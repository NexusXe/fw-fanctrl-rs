#include "fw-fanctrl-rs.h"
#include <stdio.h>

/* Statics are persistent between calls, but not between refreshes.
In the future, changing a curve will only cause a refresh, not a restart,
keeping STATE but losing statics.

Generally speaking, it is preferred to use STATE for ease of debugging. */
static bool flip_flop = false;

/*
 * The plugin entry point called by the `fw-fanctrl-rs` host.
 */
__attribute__((visibility("default"))) PluginDecision
get_decision(const PluginCallData *data) {
  /* Demonstration of using the persistent state storage:
  Keeping track of how many times this plugin has been called. */
  const char *counter_key = "call_count";
  uint64_t call_count = 0;

  // Try to get the existing count
  if (GET_STATE(data, counter_key, &call_count)) {
    call_count++;
    printf("[PLUGIN]: Call #%llu\n", (unsigned long long)call_count);
  } else {
    call_count = 1; // Default if not found or size mismatch
  }

  printf("[PLUGIN]: Time since last poll: %dms\n",
         data->ancillary.time_since_last_poll_ms);

  // Save the new count back to the persistent state
  SET_STATE(data, counter_key, call_count);

  uint16_t run_again_in = (call_count % 10 == 0) ? 50 : 500;
  if (run_again_in == 50) {
    printf("[PLUGIN]: Requesting fast poll (50ms) for call #%llu\n",
           (unsigned long long)call_count);
  }

  uint8_t num_sensors = data->ancillary.num_sensors;

  // If no sensors are reported, or the highest temp is 0, return a PWM above
  // 100 as an indication of error. The host will disable the plugin and rely
  // directly on the LUT.
  if (num_sensors == 0 || data->ancillary.highest_temp == 0) {
    return make_set_speed(255, run_again_in);
  }

  flip_flop = !flip_flop;
  if (flip_flop) {
    /* Demonstration of directly setting a particular speed
    (of course, this can be any value; a constant 50 here is just an example) */
    return make_set_speed(50, run_again_in);
  } else {
    /* Demonstration of returning an adjusted temperature to be used with the
    current fan curve */

    // Average the reading of the first `num_sensors` sensors.
    long total_temp = 0;
    for (uint8_t i = 0; i < num_sensors; i++) {
      /* Temps array represents valid temperatures
      between 0 to 251 (Raw EC output) */
      total_temp += (*data->temps)[i];
    }

    uint8_t average = total_temp / num_sensors;
    uint8_t highest = data->ancillary.highest_temp;
    uint8_t midpoint = (highest + average) / 2;

    return make_curve_speed(midpoint, run_again_in);
  }
}
