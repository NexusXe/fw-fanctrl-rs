#ifndef FW_FANCTRL_H
#define FW_FANCTRL_H

#include <assert.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define EC_TEMP_SENSOR_ENTRIES 16
#define MAX_EC_TEMP_SENSOR_VALUE 251
/* Temperature in Kelvin is stored as `value - EC_TEMP_SENSOR_OFFSET`.
 * This allows reporting a temperature range of 200K to 454K = -73C to 181C. */
#define EC_TEMP_SENSOR_OFFSET 200

// Represents the temperature sensor information returned by the EC.
typedef struct __attribute__((packed)) {
  char sensor_name[32];
  uint8_t sensor_type;
} EcResponseTempSensorGetInfo;

// A 16-element array containing the temperatures reading from the sensors.
typedef uint8_t TempSensorVector[EC_TEMP_SENSOR_ENTRIES];

// Status returned by the plugin `get` state method.
typedef enum {
  // The buffer was large enough and the data was successfully copied.
  PLUGIN_GET_STATUS_SUCCESS = 0,
  // The key does not exist or arguments were null.
  PLUGIN_GET_STATUS_NOT_FOUND = 1,
  /* The key exists, but the provided buffer was null or too small.
  The required size has been written to the length out-parameter. */
  PLUGIN_GET_STATUS_BUFFER_TOO_SMALL = 2,
  // Some other error occurred.
  PLUGIN_GET_STATUS_SOME_OTHER_ERROR = 3,
} PluginGetStatus;

// State methods provided to the plugin to persist data.
typedef struct {
  bool (*set)(const char *key, const uint8_t *data, size_t len);
  PluginGetStatus (*get)(const char *key, uint8_t *buffer, size_t *buffer_len);
} PluginStateMethods;

// Ancillary information provided to the plugin.
typedef struct {
  uint16_t time_since_last_poll_ms;
  uint8_t highest_temp;
  uint8_t num_sensors;
  uint8_t lut_speed;
} AncillaryInfo;

// Data passed to the plugin on each call.
typedef struct {
  const EcResponseTempSensorGetInfo *sensors;
  const TempSensorVector *temps;
  const PluginStateMethods *state;
  AncillaryInfo ancillary;
} PluginCallData;

// Tag representing the decision of the plugin.
typedef enum {
  // Set the fan speed directly to the given value.
  DECISION_VALUE_SET_SPEED = 0,
  // Get the fan speed from the current curve, using the given temperature.
  DECISION_VALUE_GET_SPEED_FROM_CURVE = 1,
} DecisionValueTag;

// The decision value returned by the plugin.
typedef struct {
  DecisionValueTag tag;
  union {
    struct {
      uint8_t speed_percent;
    };
    struct {
      uint8_t temperature;
    };
  };
} PluginDecisionValue;

// The final decision structure returned by the plugin.
typedef struct {
  /* The decision from the plugin; either set the fan speed directly, or get it
  from the current curve. */
  PluginDecisionValue value;
  /* Run again in `run_again_in` milliseconds. If `0`, run again after the
  configured sleep duration. */
  uint16_t run_again_in;
} PluginDecision;

// Helper for setting a specific fan speed
static inline PluginDecision make_set_speed(uint8_t speed, uint16_t delay) {
  return (PluginDecision){
      .value = {.tag = DECISION_VALUE_SET_SPEED, .speed_percent = speed},
      .run_again_in = delay};
}

// Helper for getting a value from the current curve
static inline PluginDecision make_curve_speed(uint8_t temp, uint16_t delay) {
  return (PluginDecision){.value = {.tag = DECISION_VALUE_GET_SPEED_FROM_CURVE,
                                    .temperature = temp},
                          .run_again_in = delay};
}

// Constructs a PluginDecision that will cause the host to disable the plugin.
#define MAKE_ERROR_SPEED(value)                                                \
  ({                                                                           \
    static_assert((value) > 100, "Error speed must be > 100");                \
    make_set_speed(255, 1);                                                    \
  })

// Helper for getting a value from the plugin state
static inline bool get_state(const PluginCallData *data, const char *key,
                             void *out_val, size_t expected_size) {
  size_t len = expected_size;
  PluginGetStatus status = data->state->get(key, (uint8_t *)out_val, &len);
  return (status == PLUGIN_GET_STATUS_SUCCESS && len == expected_size);
}

#define GET_STATE(data_ptr, key, out_ptr)                                      \
  get_state((data_ptr), (key), (out_ptr), sizeof(*(out_ptr)))

#define SET_STATE(data_ptr, key, value)                                        \
  (data_ptr)->state->set((key), (const uint8_t *)&(value), sizeof(value))

// Helper for converting EC temperature units to Kelvin
static inline uint16_t ec_to_kelvin(uint8_t ec_temp) {
  return (uint16_t)ec_temp + EC_TEMP_SENSOR_OFFSET;
}

// Helper for converting EC temperature units to Celsius
static inline int16_t ec_to_celsius(uint8_t ec_temp) {
  return ec_to_kelvin(ec_temp) - 273;
}

// Helper for converting Celcius to Kelvin
static inline uint16_t celsius_to_kelvin(int16_t celsius) {
  return (uint16_t)(celsius + 273);
}

// Helper for converting Kelvin to EC temperature units
static inline uint8_t kelvin_to_ec(uint16_t kelvin) {
  return (uint8_t)(kelvin - EC_TEMP_SENSOR_OFFSET);
}

// Helper for converting Kelvin to Celsius
static inline int16_t kelvin_to_celsius(uint16_t kelvin) {
  return kelvin - 273;
}

// Helper for converting Celsius to EC temperature units
static inline uint8_t celsius_to_ec(int16_t celsius) {
  return kelvin_to_ec(celsius_to_kelvin(celsius));
}

// The function signature that your plugin must implement and expose.
PluginDecision get_decision(const PluginCallData *data);

#endif // FW_FANCTRL_H
