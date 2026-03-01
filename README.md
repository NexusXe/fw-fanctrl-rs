# fw-fanctrl-rs
![License](https://img.shields.io/badge/License-AGPL--3.0--or--later-blue.svg)

## Platforms
[![Static Badge](https://img.shields.io/badge/Arch%20Linux-1793D1?style=flat&logo=archlinux&logoColor=FFFFFF&label=Platform&link=https%3A%2F%2Faur.archlinux.org%2Fpackages%2Ffw-fanctrl-rs-git)](https://aur.archlinux.org/packages/fw-fanctrl-rs-git)

## Description

A lightweight, pure-Rust daemon for custom fan control on Framework laptops.

This project interfaces directly with the ChromeOS Embedded Controller (`/dev/cros_ec`) via ioctl commands to provide responsive, customizable thermal management. It serves as a highly performant, compiled alternative to script-based fan controllers.

## Disclaimer

I am not responsible for any damage (physical, social, geopolitical, or otherwise) that may result from the use of this software.

This software has only been tested on:
 - Framework 16 (AMD Ryzen AI 300 Series)
 - Framework 13 (AMD Ryzen AI 300 Series)

Might it work on other Framework laptops? Maybe! I don't know! I don't have other Framework laptops! Heck, it might even work on Chromebooks! Who knows! You are, of course, more than welcome to try it out and report back, and maybe even open a pull request to support your device.

## Features

* **Written in Rust**: A compiled binary with fast lookups and a small system footprint.

* **Direct EC Communication**: Direct communication with the ChromeOS Embedded Controller, so no reliance on `ectool` or spawning of subprocesses.

* **Custom Fan Curves**: Configure custom temperature-to-PWM mappings.

* **Systemd Integration**: Comes with a [systemd service file](fw-fanctrl.service) that can be [enabled and started](#systemd-usage) on user login.

* **Live Reloading**: Dynamically apply new fan curves and configurations [via the CLI](#applying-a-curve).

## Installation

### Arch Linux (AUR)

The recommended installation method for Arch-based distributions is via the Arch User Repository (AUR).

Using an AUR helper like [paru](https://github.com/Morganamilo/paru):

```bash
paru -S fw-fanctrl-rs-git
```

### Manual Installation

This project requires the nightly Rust toolchain.

```bash
git clone https://github.com/NexusXe/fw-fanctrl-rs.git
cd fw-fanctrl-rs
cargo +nightly build --release
sudo make install
```

#### Systemd Usage

The daemon is managed via an instantiated systemd service tied to your user ID.
<details>
<summary><b>bash/zsh</b></summary>
To enable and start the service on boot:

```bash
sudo systemctl enable --now fw-fanctrl@$(id -u).service
```

To check the current status of the daemon:

```bash
systemctl status fw-fanctrl@$(id -u).service
```
</details>
<details>
<summary><b>fish</b></summary>
To enable and start the service on boot:

```fish
sudo systemctl enable --now fw-fanctrl@(id -u).service
```

To check the current status of the daemon:

```fish
systemctl status fw-fanctrl@(id -u).service
```
</details>

## Configuration

All configuration files are stored globally in /etc/fw-fanctrl-rs/.

**Main Configuration**: `/etc/fw-fanctrl-rs/config.toml`

For a breakdown of the `config.toml` format, just [look at it](config.toml), I believe in you.

**External Fan Curves**: `/etc/fw-fanctrl-rs/curves/`

For a breakdown of the `.curvedef` format, see the [example curve](curves/example-curve.curvedef).

## CLI Usage

### Applying a Curve

You can change the active fan curve using the CLI. This command updates your config.toml and safely restarts the systemd daemon to apply the changes immediately:
```bash
sudo fw-fanctrl-rs --use-default quiet
```

or to only change it for this session:

```bash
sudo fw-fanctrl-rs --use quiet
```

### Other CLI Commands

See `fw-fanctrl-rs --help` for a full list of commands.

## Logging

Because the daemon runs under systemd, all standard output and error streams are automatically captured by `systemd-journald`. You can view the real-time logs using `journalctl`:

```bash
sudo journalctl -u fw-fanctrl@$(id -u).service -f
```

## Motivation

I've been using [fw-fanctrl](https://github.com/TamtamHero/fw-fanctrl) for a while now, and it gets the job done. However, having a Python script constantly looping in the background has always bugged me, so I decided to rewrite it in Rust. In addition, instead of spawning new processes every tick to read temps and set fan speeds, this program uses ioctls to communicate with the EC directly, which is hypothetically better.

I will note that this program has precisely no hysteresis compensation whatsoever, which has been fine for my use cases. Feel free to make a PR if you want to add it.

### Why Rust?

I dunno. I like Rust. It's the programming language I know best. Being a compiled language, it should be faster than the original Python script. Also, Rust is becoming more and more involved in the Linux kernel, so maybe I'll be able to turn this into a kernel module someday.

## Acknowledgements

The default external fan curves provided in this package were inspired by the thermal profiles from the original [fw-fanctrl](https://github.com/TamtamHero/fw-fanctrl) project, which is licensed under the [BSD 3-Clause License](https://github.com/TamtamHero/fw-fanctrl/blob/main/LICENSE).

## License

This project is licensed under the GNU Affero General Public License v3.0 or later. See [LICENSE](LICENSE) for more details.

## Contributing

Feel free to open an issue or make a pull request. I'm not picky.
