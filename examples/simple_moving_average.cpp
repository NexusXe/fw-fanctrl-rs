#include <cstdint>

// Only this included needs to be extern "C", since the get_decision function
// prototype is declared in the header.
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
  const char *state_key = "sma";
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

  return make_curve_speed(avg_temp, 0);
}
