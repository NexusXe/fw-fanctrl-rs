use crate::{
    fan_curve::FanProfile,
    info, infov,
    temp::{CelsiusTemp, ValidEcTemp},
    warn,
};

use icy_sixel::sixel_encode;
use plotters::{backend, prelude::*};

use std::{
    io::{self, Read, Write},
    os::fd::{AsRawFd, BorrowedFd},
    path::Path,
};

const COLORS: [RGBColor; 7] = [BLACK, BLUE, CYAN, GREEN, MAGENTA, RED, YELLOW];
const FONT_TO_USE: &str = "Noto Sans";
const WIDTH: u32 = 1000;
const HEIGHT: u32 = 800;

#[derive(Debug, Default)]
pub(crate) struct TerminalSupport {
    sixel: bool,
    kitty: bool,
}

/// Queries the terminal for Sixel and Kitty graphics protocol support.
fn check_terminal_support() -> Result<TerminalSupport, Box<dyn std::error::Error>> {
    use nix::sys::termios::{LocalFlags, SetArg, tcgetattr, tcsetattr};

    let mut stdin = io::stdin();
    let mut stdout = io::stdout();
    let raw_fd = stdin.as_raw_fd();

    // switch to raw mode so we can read byte-by-byte with a timeout.
    // SAFETY: `raw_fd` is valid for the lifetime of `stdin`, which outlives all uses below.
    let mut termios = tcgetattr(unsafe { BorrowedFd::borrow_raw(raw_fd) })?;
    let original_termios = termios.clone();
    termios
        .local_flags
        .remove(LocalFlags::ICANON | LocalFlags::ECHO);
    termios.control_chars[nix::libc::VMIN] = 0;
    termios.control_chars[nix::libc::VTIME] = 2; // 200 ms fallback timeout

    tcsetattr(
        unsafe { BorrowedFd::borrow_raw(raw_fd) },
        SetArg::TCSANOW,
        &termios,
    )?;

    // since terminals without kitty support will ignore the kitty query, we can send both
    // queries back-to-back
    stdout.write_all(b"\x1b_Ga=q\x1b\\")?; // might get a response
    stdout.write_all(b"\x1b[c")?; // definitely get a response
    stdout.flush()?;

    // read both responses
    let mut all_bytes: Vec<u8> = Vec::new();
    let mut buf = [0u8; 1];
    while let Ok(1) = stdin.read(&mut buf) {
        all_bytes.push(buf[0]);
        if buf[0] == b'c' && all_bytes.windows(3).any(|w| w == b"\x1b[?") {
            break;
        }
    }

    // restore terminal settings
    tcsetattr(
        unsafe { BorrowedFd::borrow_raw(raw_fd) },
        SetArg::TCSANOW,
        &original_termios,
    )?;

    let response_str = String::from_utf8_lossy(&all_bytes);

    // any APC response from the terminal means it supports the kitty protocol
    let kitty = response_str.contains("\x1b_G");
    infov!(
        "Terminal does{} support Kitty graphics",
        if kitty { "" } else { " not" }
    );

    // parse DA1 for Sixel (parameter `4`)
    // the DA1 response looks like `\x1b[?<p1>;<p2>;...c`
    let sixel = response_str
        .find("\x1b[?")
        .and_then(|start| {
            let rest = &response_str[start + 3..];
            rest.find('c').map(|end| &rest[..end])
        })
        .is_some_and(|inner| inner.split(';').any(|p| p == "4"));

    infov!(
        "Terminal does{} support Sixel graphics",
        if sixel { "" } else { " not" }
    );

    Ok(TerminalSupport { sixel, kitty })
}

/// plots the fan curves to a file and attempts to display them in the terminal
#[allow(clippy::too_many_lines)] // too bad!
pub(super) fn plot_curves(
    to_file: &Path,
    profiles: &[FanProfile],
    force_sixel: bool,
    force_kitty: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = vec![0; (WIDTH * HEIGHT * 3) as usize];

    let support = match check_terminal_support() {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to check terminal graphics support: {}", e);
            TerminalSupport::default()
        }
    };

    let lowest_temp: CelsiusTemp =
        ValidEcTemp(profiles.iter().map(|p| p.start).min().unwrap()).to_celsius();
    let highest_temp: CelsiusTemp =
        ValidEcTemp(profiles.iter().map(|p| p.end).max().unwrap()).to_celsius();

    // use a macro because BitmapBackend and SVGBackend aren't dyn-compatible...
    // I don't like it either
    macro_rules! draw_chart {
        ($root:expr) => {{
            let root = $root;
            root.fill(&WHITE)?;

            let mut chart = ChartBuilder::on(&root)
                .caption("Fan Curves", (FONT_TO_USE, 40).into_font())
                .margin(15)
                .x_label_area_size(40)
                .y_label_area_size(40)
                .build_cartesian_2d(
                    f32::from(lowest_temp.0)..f32::from(highest_temp.0),
                    0.0_f32..100.0_f32,
                )?;

            chart
                .configure_mesh()
                .x_desc("Temperature (°C)")
                .y_desc("Fan Speed (%)")
                .axis_desc_style((FONT_TO_USE, 15))
                .draw()?;

            // allow 21 uniquely-identifiable curves before repeating
            for (i, profile) in profiles.iter().enumerate() {
                let color = COLORS[i % COLORS.len()];
                let style_index = i / COLORS.len(); // 0 = solid, 1 = dashed, 2 = dotted

                let points: Vec<(f32, f32)> = (u8::MIN..=u8::MAX)
                    .filter_map(|raw| crate::temp::UnvalidatedEcTemp(raw).get().ok())
                    .map(|ec_temp| {
                        let celsius = f32::from(ec_temp.to_celsius().0);
                        (celsius, f32::from(profile.get_fan_speed(ec_temp)))
                    })
                    .collect();

                let stroke = ShapeStyle::from(color).stroke_width(2);

                match style_index % 3 {
                    0 => chart.draw_series(LineSeries::new(points, stroke))?,
                    1 => chart.draw_series(DashedLineSeries::new(points, 10, 5, stroke))?,
                    _ => chart.draw_series(DottedLineSeries::new(points, 0, 10, move |coord| {
                        Circle::new(coord, 3, stroke.filled())
                    }))?,
                }
                .label(&*profile.name)
                .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], stroke));

                // overlay a filled dot at each explicit set point in the LUT
                let setpoints: Vec<(f32, f32)> = profile
                    .lut
                    .iter()
                    .enumerate()
                    .map(|(idx, &speed)| {
                        let ec_raw = profile.start.saturating_add(idx as u8);
                        let celsius = f32::from(ValidEcTemp(ec_raw).to_celsius().0);
                        (celsius, f32::from(speed))
                    })
                    .collect();

                chart.draw_series(
                    setpoints
                        .into_iter()
                        .map(|pt| Circle::new(pt, 2.75, stroke.filled())),
                )?;
            }

            chart
                .configure_series_labels()
                .label_font((FONT_TO_USE, 25))
                .legend_area_size(40)
                .background_style(WHITE.mix(0.8))
                .border_style(BLACK)
                .position(SeriesLabelPosition::UpperLeft)
                .draw()?;
        }};
    }

    if to_file.extension().is_some_and(|ext| ext == "svg") {
        infov!("Drawing SVG fan curve plot to {}", to_file.display());
        draw_chart!(SVGBackend::new(to_file, (WIDTH, HEIGHT)).into_drawing_area());
    }
    // still need to draw to buffer for kitty/sixel output
    infov!("Drawing fan curve plot to buffer for kitty/sixel output");
    draw_chart!(
        BitMapBackend::<backend::RGBPixel>::with_buffer_and_format(&mut buffer, (WIDTH, HEIGHT))?
            .into_drawing_area()
    );

    if to_file.extension().is_some_and(|ext| ext != "svg") {
        info!("Saving fan curve plot to {}...", to_file.display());
    }

    let using_kitty = (support.kitty || force_kitty) && !force_sixel;
    let using_sixel = (support.sixel && !support.kitty) || force_sixel;

    // encode to PNG in memory; clone first since from_raw consumes the buffer
    // and sixel needs the original raw RGB pixels
    let raw_pixels = buffer.clone();
    let mut png_buf: Vec<u8> = Vec::new();
    let image = image::RgbImage::from_raw(WIDTH, HEIGHT, buffer)
        .ok_or("buffer dimensions do not match image size")?;

    if using_kitty || to_file.extension().is_some_and(|ext| ext == "png") {
        // encode to png in memory
        infov!("Encoding plot to png in memory for kitty/png output");
        image.write_to(
            &mut std::io::Cursor::new(&mut png_buf),
            image::ImageFormat::Png,
        )?;
        if to_file.extension().is_some_and(|ext| ext == "png") {
            // actually write the png to disk
            infov!("Writing png to {}", to_file.display());
            std::fs::write(to_file, &png_buf)?;
        }
    } else if to_file.extension().is_some_and(|ext| ext != "svg") {
        // otherwise we also need to encode to whatever other format the user wants
        infov!("Writing plot to {}", to_file.display());
        image.save(to_file)?;
    }

    // prefer kitty graphics over sixel
    if using_kitty {
        kitty(&png_buf)?;
    }
    if using_sixel {
        sixel(&raw_pixels, WIDTH, HEIGHT)?;
    }
    if !using_kitty && !using_sixel {
        infov!("Terminal doesn't seem to support Sixel or Kitty graphics.");
    }
    println!("[OUT]: Image saved to {}", to_file.display());
    Ok(())
}

/// Helper to send a PNG as Kitty graphics
fn kitty(png_bytes: &[u8]) -> io::Result<()> {
    use base64::{Engine as _, engine::general_purpose};

    const CHUNK_SIZE: usize = 4096;
    let encoded = general_purpose::STANDARD.encode(png_bytes);
    let mut chunks = encoded.as_bytes().chunks(CHUNK_SIZE).peekable();
    let mut stdout = io::stdout().lock();
    let mut is_first = true;

    infov!("Sending {:} bytes as Kitty graphics", png_bytes.len());

    info!("Curves:");
    while let Some(chunk) = chunks.next() {
        // m=1 -> "more data coming"; m=0 -> "last chunk"
        let m = u8::from(chunks.peek().is_some());

        if is_first {
            // a=T: transmit and display immediately
            // f=100: the payload is a PNG file
            write!(stdout, "\x1b_Ga=T,f=100,m={m};")?;
            is_first = false;
        } else {
            write!(stdout, "\x1b_Gm={m};")?;
        }

        stdout.write_all(chunk)?;
        write!(stdout, "\x1b\\")?; // string terminator
    }

    // newline so the terminal prompt doesn't overwrite the image
    writeln!(stdout)?;
    stdout.flush()?;

    Ok(())
}

/// Helper to send RGB bytes as Sixel graphics
fn sixel(rgb_bytes: &[u8], width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    use icy_sixel::{EncodeOptions, QuantizeMethod};
    use image::{ImageBuffer, Rgba, buffer::ConvertBuffer};
    let options = EncodeOptions {
        max_colors: 256,
        diffusion: 0.0, // disable dithering
        quantize_method: QuantizeMethod::Wu,
        ..
    };

    // icy_sixel needs RGBA, so convert :(
    infov!("Converting {}x{} RGB to RGBA", width, height);
    let rgba_image: ImageBuffer<Rgba<u8>, Vec<_>> =
        image::RgbImage::from_vec(width, height, rgb_bytes.to_vec())
            .ok_or("buffer dimensions do not match image size")?
            .convert();
    infov!("Done.");

    let sixel_string = sixel_encode(&rgba_image, width as usize, height as usize, &options)?;
    info!("Curves:");
    println!("{sixel_string}");

    Ok(())
}
