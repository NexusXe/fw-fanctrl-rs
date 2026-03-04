#include <cstdint>

extern "C" {
#include "fw-fanctrl-rs.h"

struct MovingAverageState {
  uint8_t max_temps[5];
  uint8_t head;
  uint8_t count;
};

__attribute__((visibility("default"))) PluginDecision
get_decision(const PluginCallData *data) {
  const char *state_key = "sma_history";
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
  uint32_t sum_temp = 0;
  for (int i = 0; i < state.count; i++) {
    sum_temp += state.max_temps[i];
  }

  uint8_t avg_temp = sum_temp / state.count;

  // As current max temperature increases compared to the average,
  // put more weight into the max temperature.
  uint32_t total_weight = state.count;
  uint32_t weighted_temp = sum_temp;

  if (current_temp > avg_temp) {
    // diff represents how much hotter it is than the average
    uint8_t diff = current_temp - avg_temp;

    // Add additional weight for the current maximum temperature proportional to
    // diff
    uint32_t extra_weight = diff;
    total_weight += extra_weight;
    weighted_temp += current_temp * extra_weight;
  }

  uint8_t adjusted_temp = weighted_temp / total_weight;

  return make_curve_speed(adjusted_temp, 0);
}

} // extern "C"
