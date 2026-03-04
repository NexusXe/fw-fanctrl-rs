#Developing Plugins for fw - fanctrl - rs

`fw-fanctrl-rs` supports extending fan control logic using C-ABI compatible shared object (`.so`) plugins. By providing a compiled library that implements a specific interface, `fw-fanctrl-rs` will delegate its fan control decisions to your code on every polling interval.

This document explains the C interface provided in [`plugins.rs`](/src/fan_curve/plugins.rs) and the companion C headers in [`fw-fanctrl-rs.h`](fw-fanctrl-rs.h).

---

## Quick Start

1. Include the `include/fw-fanctrl-rs.h` header in your C project.
2. Implement the `get_decision` function with default visibility.
3. Compile your code as a shared object library (`.so`).
4. Configure `fw-fanctrl-rs` to load your built plugin.

Here's a minimal example:

```c
#include "fw-fanctrl-rs.h"

__attribute__((visibility("default"))) PluginDecision
get_decision(const PluginCallData *data) {
  // If the highest temperature across all sensors is above 80°C, set the fan to
  // 100%
  if (data->ancillary.highest_temp >= 80) {
    return make_set_speed(100, 200); // 100% speed, poll again in 200ms
  }

  // Otherwise, redirect to the normal active fan curve, evaluating as if the
  // highest temperature was 50°C
  return make_curve_speed(50, 500); // Poll again in 500ms
}
```

    Compile it with :
```sh gcc -
    shared - fPIC -
    o my_plugin.so my_plugin
        .c
``` <br>

    ##Interface The program passes the state of the system and
        sensors via the `PluginCallData` struct to the `get_decision` function
            upon every execution cycle.

```c PluginDecision
            get_decision(const PluginCallData *data);
```

    ## #1. `PluginCallData`

    This structure contains all available information regarding the system's thermal state.

```c typedef struct {
  const EcResponseTempSensorGetInfo *sensors;
  const TempSensorVector *temps;
  const PluginStateMethods *state;
  AncillaryInfo ancillary;
} PluginCallData;
```

#### Sensor Details
- `sensors`: An array holding metadata (such as name and type) for up to 16 sensors. 
- `temps`: The active temperature readings for each sensor in the array. Note that temperature is provided as a raw `uint8_t` where `Kelvin = reading - 200` corresponding to a theoretical span of 200K (-73°C) to 454K (+181°C).

#### Ancillary Info
Provides summarized information:
```c
typedef struct {
  uint16_t time_since_last_poll_ms; // Time since last poll in milliseconds
                                    // (coarse timing)
  uint8_t highest_temp; // The highest recorded temperature natively reported
  uint8_t num_sensors;  // The total number of registered sensors
  uint8_t lut_speed;    // The current Look-Up Table target speed
} AncillaryInfo;
```

---

### 2. Returning a Decision

Your logic must return a `PluginDecision`. `fw-fanctrl-rs.h` provides two helper macros to construct decisions easily:

- `make_set_speed(uint8_t speed_percent, uint16_t delay)`: Sets the fan speed directly to `speed_percent` (0-100%).
- `make_curve_speed(uint8_t synthetic_temperature, uint16_t delay)`: Looks up the speed on the *currently active* user-configured fan curve using the `synthetic_temperature` you provide.

**Delay (`run_again_in`):**  
Both helpers take a `delay` parameter in milliseconds indicating when `fw-fanctrl-rs` should call the plugin again. If you provide `0`, the standard configured system poll interval is used.

---

### 3. Persistent State Storage

Because the library dynamically unloads and reloads, standard C statics or global variables could be lost during a hard reboot or configuration refresh (though they generally persist across direct calls). 

To cleanly persist data between multiple calls, `fw-fanctrl-rs` exposes thread-safe storage methods. You can access these methods via the `PluginStateMethods` inside `PluginCallData`.

#### Storing and Retrieving State
We provide `GET_STATE` and `SET_STATE` macros for convenience:

```c
uint64_t my_counter = 0;

// Try to grab existing state into `my_counter`
if (GET_STATE(data, "call_count", &my_counter)) {
  // Successfully found!
  my_counter++;
} else {
  // Key not found or mismatched size
  my_counter = 1;
}

// Persist the counter
SET_STATE(data, "call_count", my_counter);
```

    The `PluginStateMethods` struct exposes the underlying operations
        as function pointers :

```c typedef struct {
  bool (*set)(const char *key, const uint8_t *data, size_t len);
  PluginGetStatus (*get)(const char *key, uint8_t *buffer, size_t *buffer_len);
} PluginStateMethods;
```
If you need tighter control or need to manually track buffer sizes, you can directly use `data->state->set` and `data->state->get`.

> Check out the [`fw-fanctrl-rs.h`](fw-fanctrl-rs.h) for more information, or the [`example-plugin.c`](/examples/example-plugin.c) for a working example.

## Safety

- Any function you provide can do Literally Whatever You Want It To, with the exception of throwing an exception or panicking. Since it's across the FFI boundary, errors are #UB. If you make my daemon do #UB, I will Get You.

- **REMEMBER** that the plugin is running as ROOT! Don't do anything you wouldn't do as root, and if you do, make sure you buy the kernel dinner first.
