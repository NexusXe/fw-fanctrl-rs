import subprocess
import csv
import sys
import itertools

try:
    import matplotlib.pyplot as plt
    plt.rcParams['font.family'] = 'Noto Sans'
except ImportError:
    print("Matplotlib is required to plot. Please install it with 'pip install matplotlib'.")
    sys.exit(1)

# List of profiles to plot
profiles = [
    "fw-laziest", "fw-lazy", "fw-medium",
    "fw-deaf", "fw-aeolus", "default", "quiet", "performance", "turbo", "deaf"
]

# Globally disable antialiasing for lines, axes edges, and ticks
plt.rcParams['lines.antialiased'] = False
plt.rcParams['patch.antialiased'] = False
plt.rcParams['text.antialiased'] = False

plt.figure(figsize=(12, 8))

# Build first to avoid compilation output mixing into our subprocess output and slowing things down
print("Building rust binary...")
subprocess.run(["cargo", "build", "--quiet"], check=True)

# Define different line styles to help differentiate profiles
line_styles = itertools.cycle(['-', '--', '-.', ':'])

for profile in profiles:
    print(f"Fetching data for '{profile}'...")
    try:
        result = subprocess.run(
            ["cargo", "run", "--quiet", "--", "-p", profile, "--curve"], 
            capture_output=True, text=True, check=True
        )
        
        # Parse CSV output
        lines = result.stdout.strip().split('\n')
        reader = csv.reader(lines)
        header = next(reader)
        
        temps = []
        pwms = []
        for row in reader:
            temps.append(int(row[0]))
            pwms.append(int(row[1]))
            
        plt.plot(temps, pwms, label=profile, linestyle=next(line_styles), antialiased=False)
    except subprocess.CalledProcessError as e:
        print(f"Failed to get data for profile {profile}: {e}")
        print(f"Stderr: {e.stderr}")

plt.xlabel("Temperature (°C)", fontsize=12)
plt.ylabel("Fan Speed (%)", fontsize=12)
plt.title("fw-fanctrl-rs fan curves", fontsize=16, pad=15, fontfamily="Noto Sans")
plt.legend(bbox_to_anchor=(1.05, 1), loc='upper left')
plt.grid(True, linestyle='--', alpha=0.7)

# Typical temperature range of interest for laptops
plt.xlim(20, 105) 
plt.ylim(0, 105)

output_file = "fan_curves_plot.webp"
plt.savefig(output_file, dpi=300, bbox_inches='tight', pil_kwargs={'quality': 100, 'method': 6, 'lossless': True})
print(f"\nSuccessfully saved plot to {output_file}")

try:
    plt.show()
except Exception:
    pass
