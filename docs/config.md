# Scenario configuration

Every field below lives in `wireplant_config::schema`. A scenario that omits a
section gets that struct's `Default`, and a scenario that sets a field the
runtime refuses fails validation before any listener is bound — see
`wireplant_config::validation`.

## `[facility]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `name` | string | required | Must not be blank. Used for the OPC-UA namespace and the MQTT topic. |
| `description` | string | none | Free text, carried into the OPC-UA node descriptions. |
| `tick_rate_ms` | integer | 100 | Physics interval. Must be between 1 and 10000. |

## `[memory]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `min_table_registers` | integer | 128 | Floor for each of the four tables. Raising it costs memory and response size on `GET /api/v1/state`. |
| `padding_registers` | integer | 8 | Room past the highest declared address. |

Each table is sized `max(min_table_registers, highest_address + register_span + padding_registers)`.
The floor exists so a SCADA client polling an address no tag declares gets a zero rather than an
out-of-range exception.

## `[network]`

All four sections are optional. A protocol with `enabled = false` is not bound, so its address
cannot fail startup, and its `bind_addr` may be left out.

### `[network.modbus]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `true` | Modbus TCP listener enable flag |
| `bind_addr` | string | `"0.0.0.0:5020"` | Modbus TCP socket bind address |
| `default_unit_id` | integer | `1` | Default slave address reported in logs |
| `allowed_unit_ids` | array of integers | `[]` | Allowed slave IDs; empty answers any unit ID |
| `max_connections` | integer | `100` | Connection limit; clients past limit are refused |
| `serial_enabled` | bool | `false` | Modbus RTU serial line enable flag |
| `serial_port` | string | none | Serial device path (e.g. `"/dev/ttyUSB0"`, `"COM1"`) |
| `baud_rate` | integer | `9600` | Serial transmission speed (e.g. 9600, 19200, 115200) |
| `parity` | string | `"none"` | Serial parity: `"none"`, `"even"`, `"odd"` |
| `data_bits` | integer | `8` | Character data bits (7 or 8) |
| `stop_bits` | integer | `1` | Character stop bits (1 or 2) |

### `[network.opcua]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `true` | OPC-UA server enable flag |
| `bind_addr` | string | `"0.0.0.0:4840"` | OPC-UA TCP listen endpoint |
| `allow_anonymous` | bool | `true` | Allow connections without user credentials |
| `security_policies` | array of strings | `["Basic256Sha256"]` | Supported policies: `"Basic256Sha256"`, `"Aes128Sha256RsaOaep"` |
| `users` | array of tables | `[]` | Username / password credential pairs |
| `pki_dir` | string | `"./pki"` | Directory for PKI certificate and private key store |

### `[network.http]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `true` | HTTP REST API enable flag |
| `bind_addr` | string | `"0.0.0.0:8080"` | HTTP listen endpoint |
| `api_key` | string | none | Authorization token; if set, `/api/v1/*` routes require `Authorization: Bearer <token>` or `X-API-Key: <token>`. `/health` remains public. |

### `[network.mqtt]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `false` | MQTT telemetry publisher enable flag |
| `broker_addr` | string | `"127.0.0.1:1883"` | Broker host:port endpoint |
| `client_id` | string | none | Client ID; if omitted, derived as `wireplant_<facility_name_lowercased>` |
| `username` | string | none | Broker authentication username |
| `password` | string | none | Broker authentication password |
| `subscribe_commands` | bool | `true` | Subscribe to incoming command topics when MQTT is enabled |
| `command_topic` | string | none | Override command topic; defaults to `wireplant/<facility_slug>/command` |

## `[historian]`

Persistent time-series telemetry store backed by an embedded SQLite database running in Write-Ahead Logging (WAL) mode.

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `true` | Historian background worker enable flag |
| `db_path` | string | `"wireplant_history.db"` | SQLite database file location |
| `flush_interval_ms` | integer | `1000` | Ingestion buffer commit interval in milliseconds |
| `batch_size` | integer | `500` | Ingestion buffer capacity threshold |
| `retention_hours` | integer | `48` | Pruning retention ceiling in hours |

## `[[tags]]`

| Field | Type | Default |
|---|---|---|
| `id` | string | required, unique |
| `description` | string | none |
| `area` | `coil`, `discrete_input`, `input_register`, `holding_register` | required |
| `address` | integer | required |
| `data_type` | `boolean`, `int16`, `uint16`, `int32`, `uint32`, `float32`, `float64` | required |
| `endianness` | `big_endian`, `word_swapped_big`, `little_endian`, `word_swapped_little` | `word_swapped_big` |
| `units` | string | `""` |
| `waveform` | table | required |

A tag occupies `data_type.register_span()` registers: one for the 16-bit types and `boolean`, two
for `int32`/`uint32`/`float32`, four for `float64`. Two tags may not overlap, and a register type
aimed at a coil or discrete input is rejected.

## `waveform`

| `type` | Fields |
|---|---|
| `constant` | `value` |
| `sine` | `min`, `max`, `period_secs` |
| `ramp` | `min`, `max`, `step_per_tick` |
| `triangle` | `min`, `max`, `period_secs` |
| `random_walk` | `center`, `max_jitter`, `drift_sigma` |
| `trip_threshold` | `source_tag`, `threshold`, `inverted` (default `false`) |

## `[[processes]]`

Each process names the tags it reads and writes. An operator input must be a holding register or a
coil, because a read-only table cannot hold a command.

| `type` | Tag fields | `config` |
|---|---|---|
| `tank` | `level_tag`, `inlet_valve_tag`, `drain_pump_tag` | `surface_area_m2`, `max_height_m`, `max_inflow_m3_s`, `max_outflow_m3_s`, `gravity_drain_coeff`, `initial_level_m` |
| `thermal` | `temp_tag`, `heater_tag` | `ambient_temp_c`, `max_heating_power_watts`, `heat_capacity_j_k`, `heat_loss_coeff_w_k`, `initial_temp_c` |
| `pid_controller` | `setpoint_tag`, `process_var_tag`, `output_tag` | `kp`, `ki`, `kd`, `min_output`, `max_output`, `anti_windup` (default `true`), `derivative_filter_n` (default `10.0`), `derivative_mode` (`"on_measurement"` [default] or `"on_error"`) |

`derivative_mode = "on_measurement"` computes `-Kd * d(PV)/dt` to eliminate setpoint kick on step changes, while `"on_error"` computes classic `Kd * d(SP - PV)/dt`. `derivative_filter_n` specifies the low-pass filter roll-off coefficient (typically 5.0 to 20.0).

`initial_level_m` and `initial_temp_c` are optional. Omitting them starts the model empty and at
ambient respectively, which is different from starting at zero degrees.


## `[[fault_profiles]]`

| Field | Type | Default |
|---|---|---|
| `id` | string | required |
| `name` | string | required |
| `description` | string | none |
| `actions` | array of tables | `[]` |

Each action has `target_tag`, `force_value`, and an optional `duration_secs`. An action without a
duration stays in force until the operator clears it; with one, it lapses that many simulated
seconds later.

## `[[alarms]]`

ANSI/ISA-18.2 compliant alarm state engine configuration. Evaluates limit boundaries with deadband
hysteresis and on/off delay qualification filters, synchronizing lifecycle states directly with
Process Image coils and holding registers.

| Field | Type | Default | Notes |
|---|---|---|---|
| `id` | string | required | Unique alarm identifier. |
| `description` | string | `""` | Free-text descriptive label. |
| `source_tag` | string | required | Identifier of the tag monitored by this alarm. |
| `alarm_type` | `high`, `low`, `high_high`, `low_low` | required | Limit comparison direction. |
| `limit` | float | required | Threshold limit triggering the alarm. |
| `deadband` | float | `0.0` | Hysteresis deadband to eliminate threshold chattering. Must be non-negative. |
| `severity` | `low`, `medium`, `high`, `critical` | `medium` | ISA-18.2 alarm priority severity. |
| `on_delay_secs` | float | `0.0` | Time in seconds condition must persist before entering alarm state. |
| `off_delay_secs` | float | `0.0` | Time in seconds value must stay within normal limits before clearing. |
| `latching` | bool | `false` | When true, returning to normal transitions to Latched state until reset. |
| `max_shelve_duration_secs` | float | `86400.0` | Maximum time in seconds an alarm may be shelved before automatically unshelving. |
| `active_coil_addr` | integer | none | Coil address set to `true` whenever the alarm is active. |
| `unack_coil_addr` | integer | none | Coil address set to `true` while the alarm is unacknowledged. |
| `ack_coil_addr` | integer | none | Coil address written by an operator or SCADA client to acknowledge the alarm. |
| `reset_coil_addr` | integer | none | Coil address written to reset a latched alarm once process value clears. |
| `state_reg_addr` | integer | none | Holding register address exposing ISA-18.2 state enum: `0` (Normal), `1` (Unack), `2` (Ack), `3` (RTN Unack), `4` (Latched), `5` (Shelved). |

