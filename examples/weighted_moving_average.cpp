#include <cstdint>

extern "C" {
#include "fw-fanctrl-rs.h"
}

struct MovingAverageState {
  uint8_t max_temps[5];
  uint8_t head;
  uint8_t count;
};

__attribute__((visibility("default"))) PluginDecision
get_decision(const PluginCallData *data) {
  const char *state_key = "wma";
  MovingAverageState state = {0};

  // Attempt to load the state; if not found, state will remain 0-initialized
  GET_STATE(data, state_key, &state);

  uint8_t current_temp = data->ancillary.highest_temp;

  // If no sensors or invalid temp, return error speed
  if (data->ancillary.num_sensors == 0 || current_temp == 0) {
    return MAKE_ERROR_SPEED(255);
  }

  // Insert the current values into the ring buffer
  state.max_temps[state.head] = current_temp;

  state.head = (state.head + 1) % 5;
  if (state.count < 5) {
    state.count++;
  }

  // Save the updated state
  SET_STATE(data, state_key, state);

  // Calculate sum of history
  uint16_t sum_temp = 0;
  for (uint8_t i = 0; i < state.count; i++) {
    sum_temp += state.max_temps[i];
  }

  uint8_t avg_temp = sum_temp / state.count;

  uint8_t adjusted_temp;
  if (ec_to_celsius(current_temp) >= 90) {
    // If the temperature is 90C or above, we want to ramp the fan up faster.
    // The current temperature is already high, so we don't need to wait for
    // the average to catch up.
    adjusted_temp = current_temp;
  } else if (current_temp > avg_temp) {
    // As current max temperature increases compared to the average,
    // put more weight into the max temperature.
    uint16_t total_weight = state.count;
    uint32_t weighted_temp = sum_temp;

    // diff represents how much hotter it is than the average
    uint8_t diff = current_temp - avg_temp;

    // Add additional weight for the current maximum temperature proportional to
    // diff
    uint8_t extra_weight = diff;
    total_weight += extra_weight;
    weighted_temp += (uint32_t)current_temp * extra_weight;
    adjusted_temp = weighted_temp / total_weight;
  } else {
    adjusted_temp = avg_temp;
  }

  return make_curve_speed(adjusted_temp, 0);
}
