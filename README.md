# WirePlant

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Company](https://img.shields.io/badge/owned%20by-macrodataware-blue)](https://macrodataware.com/)

WirePlant is a single-binary simulator for OT work: it stands up a fake plant that speaks
Modbus TCP, OPC-UA, MQTT and HTTP at the same time, all reading and writing one in-memory
Process Image that a physics engine animates tick by tick. Flipping a setpoint over Modbus is
visible immediately over OPC-UA and in the MQTT stream, because in a real plant it would be.

It is a simulator, not an emulator. It does not talk fieldbus, does not pretend to be a specific
vendor's device, and will happily serve numbers a real PLC would never produce.

## Build and run

```bash
cargo build --release --bin wireplant
./target/release/wireplant --help

# Validate scenario syntax, tag allocations, and memory requirements offline:
./target/release/wireplant validate scenarios/closed_loop_water_plant.toml

# Export recorded SQLite historian telemetry to CSV or JSON offline:
./target/release/wireplant export-history --db wireplant_history.db --format csv --output telemetry.csv

# Default hydroelectric scenario, structured logs:
./target/release/wireplant --headless

# The interactive dashboard, on the closed-loop water plant:
./target/release/wireplant --tui --scenario scenarios/closed_loop_water_plant.toml
```

### Docker

A multi-stage container image is available for running WirePlant in containerized environments:

```bash
docker build -t wireplant .
docker run -d -p 5020:5020 -p 4840:4840 -p 8080:8080 wireplant
```

`--tui` and `--headless` are mutually exclusive. With neither, WirePlant prints logs and waits
for Ctrl+C.

## What it serves

| Protocol | Default | Notes |
|---|---|---|
| Modbus TCP / RTU | `0.0.0.0:5020` | FC01–FC06, FC15–FC16, FC22, FC23, FC43, FC08 against the four Process Image tables. Modbus RTU serial line supported via `serial_port`. |
| OPC-UA | `opc.tcp://0.0.0.0:4840` | Variable node per tag under `Objects/WirePlant_<Facility>`. Anonymous or user authentication, `Basic256Sha256` / `Aes128Sha256RsaOaep` encryption, client writes sync to memory. |
| HTTP | `0.0.0.0:8080` | `GET /health` (public), `GET /metrics` (Prometheus text exposition), `GET /api/v1/state`, `POST /api/v1/fault`. Optional API key authentication via `Authorization: Bearer <key>` or `X-API-Key: <key>`. |
| MQTT | off | Publishes JSON telemetry to `wireplant/<facility>/telemetry` and subscribes to actuator commands on `wireplant/<facility>/command`. Optional username and password authentication. |
| Historian | WAL SQLite | Embedded time-series telemetry store (`wireplant_history.db`) with retention pruning. Exportable via `export-history` CLI subcommand. |

Each protocol has an `enabled` flag in the scenario. A disabled protocol is not bound at all,
so its port cannot fail startup.

### HTTP API

The REST API exposes health checks, Prometheus telemetry, plant state inspection, override injection, alarm management, and historical data querying.

| Method | Endpoint | Auth | Description |
|---|---|---|---|
| `GET` | `/health` | Public | Returns service liveness, uptime, and facility name (HTTP 200 or 503). |
| `GET` | `/metrics` | Public | Prometheus exposition format (version 0.0.4) metrics scrape. |
| `GET` | `/api/v1/state` | Protected | Current plant state and decoded tag readings. Supports `?include_raw=true`. |
| `POST` | `/api/v1/fault` | Protected | Pins an override on a tag (`{"tag_id":"...","forced_value":123.4}`). |
| `DELETE` | `/api/v1/fault/{tag_id}` | Protected | Clears an active override on the specified tag. |
| `GET` | `/api/v1/alarms` | Protected | Lists all configured alarms and their current ISA-18.2 states. |
| `POST` | `/api/v1/alarms/{id}/acknowledge` | Protected | Acknowledges an active unacknowledged alarm. |
| `POST` | `/api/v1/alarms/{id}/reset` | Protected | Resets a latched alarm whose process variable has returned to normal. |
| `POST` | `/api/v1/alarms/{id}/shelve` | Protected | Shelves an alarm for a given duration (`{"duration_secs":300}`). |
| `POST` | `/api/v1/alarms/{id}/unshelve` | Protected | Unshelves a previously shelved alarm. |
| `GET` | `/api/v1/history` | Protected | Queries time-series history (`?tag_id=...&start_time_ms=...&end_time_ms=...&limit=...`). |

```bash
# Public health check:
curl -s http://127.0.0.1:8080/health
# {"status":"ok","uptime_secs":42,"facility_name":"Hydroelectric Substation 04"}

# Prometheus text exposition:
curl -s http://127.0.0.1:8080/metrics
# wireplant_uptime_seconds 42.125
# wireplant_modbus_sessions 0
# wireplant_alarms{severity="high",state="normal"} 1
# wireplant_process_tag{tag_id="penstock_pressure_psi",area="holding_register",units="PSI"} 145.200000

# Query state (pass ?include_raw=true to include raw Modbus coils and registers summaries):
curl -s -H 'Authorization: Bearer my-secret-key' 'http://127.0.0.1:8080/api/v1/state?include_raw=true'

# Force an override (pinned until cleared):
curl -X POST http://127.0.0.1:8080/api/v1/fault \
  -H 'Authorization: Bearer my-secret-key' \
  -H 'Content-Type: application/json' \
  -d '{"tag_id":"penstock_pressure_psi","forced_value":195.0}'

# Clear an active override:
curl -X DELETE http://127.0.0.1:8080/api/v1/fault/penstock_pressure_psi \
  -H 'Authorization: Bearer my-secret-key'

# Acknowledge or reset an alarm:
curl -X POST http://127.0.0.1:8080/api/v1/alarms/penstock_hi/acknowledge \
  -H 'Authorization: Bearer my-secret-key'

# Shelve an alarm for 10 minutes:
curl -X POST http://127.0.0.1:8080/api/v1/alarms/penstock_hi/shelve \
  -H 'Authorization: Bearer my-secret-key' \
  -H 'Content-Type: application/json' \
  -d '{"duration_secs":600}'

# Query historical telemetry from the embedded SQLite database:
curl -s -H 'Authorization: Bearer my-secret-key' \
  'http://127.0.0.1:8080/api/v1/history?tag_id=penstock_pressure_psi&limit=100'
```

`active_tags` holds the decoded values. `coils_summary` and `registers_summary` are the raw
tables, which are included when `?include_raw=true` is requested; their length comes from the
scenario's `[memory]` section.


### Modbus

```bash
# Registers 100 and 101 hold penstock_pressure_psi (a 32-bit float):
mbpoll -m tcp -p 5020 -r 100 -c 2 127.0.0.1

# Coil 1 is emergency_cutoff_relay:
mbpoll -m tcp -p 5020 -t 0 -r 1 -c 1 127.0.0.1
```

Unit ids are whatever the scenario says: `modbus.allowed_unit_ids` lists the ids served (empty
means any), and `modbus.max_connections` caps concurrent clients. A client past the cap is
refused rather than queued.

### MQTT

```bash
./target/release/wireplant --headless --mqtt-broker 127.0.0.1:1883 --mqtt-enabled
mosquitto_sub -h 127.0.0.1 -p 1883 -t 'wireplant/+/telemetry'

# Send a setpoint or valve command over MQTT:
mosquitto_pub -h 127.0.0.1 -p 1883 -t 'wireplant/hydroelectric_substation_04/command' \
  -m '{"tag_id":"penstock_pressure_psi","value":160.0}'
```

The telemetry payload is `{"timestamp_ms":...,"facility":"...","tick_number":42,"metrics":{...}}`.
Commands published to `wireplant/<facility>/command` are decoded and written to the Process Image.
Writes to read-only hardware inputs (discrete inputs and input registers) are refused.

## Scenarios

A scenario is one TOML file: a facility, a network section, a memory section, a list of tags, and
optionally process models and fault profiles.

```toml
[facility]
name = "Hydroelectric Substation 04"
tick_rate_ms = 100

[memory]
min_table_registers = 1000   # floor so a poll of an undeclared address answers zero
padding_registers = 8        # room past the highest declared address

[network.modbus]
bind_addr = "0.0.0.0:5020"

[network.opcua]
enabled = false

[network.http]
bind_addr = "0.0.0.0:8080"

[network.mqtt]
broker_addr = "127.0.0.1:1883"
enabled = false

[[tags]]
id = "penstock_pressure_psi"
area = "holding_register"
address = 100                # a float32 occupies 100 and 101
data_type = "float32"
endianness = "word_swapped_big"
units = "PSI"
waveform = { type = "sine", min = 120.0, max = 185.0, period_secs = 20.0 }

[[tags]]
id = "emergency_cutoff_relay"
area = "coil"
address = 1
data_type = "boolean"
waveform = { type = "trip_threshold", source_tag = "penstock_pressure_psi", threshold = 180.0 }
```

Every address is checked at load: two tags that overlap, a trip whose source tag does not exist, a
`float32` aimed at a coil, or an enabled protocol with no bind address is a startup error, not a
warning. The full field reference is in [docs/config.md](docs/config.md).

Bundled scenarios under [`scenarios/`](scenarios):

1. [`default_hydro.toml`](scenarios/default_hydro.toml) — the same plant as the embedded default.
2. [`closed_loop_water_plant.toml`](scenarios/closed_loop_water_plant.toml) — a tank whose level
   responds to valve and pump commands written over Modbus. Use this one to test a control loop
   rather than a read-only dashboard.
3. [`water_treatment.toml`](scenarios/water_treatment.toml) — municipal filtration, 15 tags.
4. [`petrochemical.toml`](scenarios/petrochemical.toml) — refinery FCCU, 16 tags.

## What the engine does

Each tick runs five passes: sample the tags a process reads as actuator inputs, advance the free
running waveforms, step the closed-loop models, evaluate cross-tag trips, then take the write lock
once and commit everything. Holding the lock only for the commit keeps protocol clients polling
during the arithmetic.

Waveforms are `constant`, `sine`, `ramp` (wraps to `min` at `max`), `triangle`, `random_walk`
(Gaussian steps clamped to a band) and `trip_threshold` (a discrete that follows another tag
across a threshold, optionally inverted).

Process models are `tank` (mass balance with Torricelli drainage), `thermal` (first-order heating
and Newton cooling) and `pid_controller` (clamped output with optional anti-windup). A process
names the tags it reads and writes, and the engine leaves operator-owned inputs alone rather than
overwriting them with a waveform.

## Registers and byte order

A Modbus register is 16 bits, so a 32-bit value takes two of them. Which two, in what order, is
vendor-specific, and getting it wrong yields a plausible number rather than an error. Encoding
`142.75` (`0x430E_C000`):

| `endianness` | Wire order | Registers |
|---|---|---|
| `big_endian` | ABCD | `430E C000` |
| `word_swapped_big` | CDAB | `C000 430E` |
| `little_endian` | DCBA | `00C0 0E43` |
| `word_swapped_little` | BADC | `0E43 00C0` |

`word_swapped_big` is the default, because it is what most Schneider Modicon gear expects.
`wireplant_core::endian` converts both ways. The table above is pinned twice: a unit test that
compares actual bytes rather than trusting a round trip, and a property test that reorders the
four raw bytes by hand and checks the encoder against that.

`TagDataType` decides the width: `boolean` and the 16-bit types take one register, `int32`/
`uint32`/`float32` take two, `float64` takes four. One function, `TagDataType::register_span`,
answers that for the validator, the codec and the memory sizing alike.

## Workspace layout

Nine crates. The protocol servers do not depend on each other — they all talk to `wireplant_core`
and nothing else, so building the Modbus server does not pull in an MQTT client.

```text
crates/
├── wireplant_core/      # Process Image, the value codec, endianness, host:port parsing
├── wireplant_engine/    # Waveforms, trips, ticker, ISA-18.2 alarms, process models, SQLite historian
├── wireplant_config/    # Scenario schema, validation, memory sizing
├── wireplant_modbus/    # Modbus TCP and RTU server (FC01–06, FC15–16, FC22, FC23, FC43, FC08)
├── wireplant_opcua/     # OPC-UA address space, encryption policies, and bi-directional sync loop
├── wireplant_mqtt/      # rumqttc publisher and JSON payload
├── wireplant_api/       # Axum REST API with auth middleware and historian query endpoint
├── wireplant_tui/       # Ratatui 7-tab industrial operations dashboard
└── wireplant_cli/       # The `wireplant` binary: clap args, validator, orchestrator, shutdown
```

## The dashboard

`--tui` opens seven tabs:
1. **Process Tags**: live tag values, engineering units, min/max/avg statistics, and trend sparklines.
2. **Alarms**: ISA-18.2 alarm monitor displaying trip state, severity, unack status, and latency timers.
3. **Control Loops**: active process models (tanks, thermal systems, PID loops) and current actuators.
4. **Network & Svc**: active sessions, protocol listeners, and communication throughput.
5. **Memory Map**: visual inspection of Coils, Discrete Inputs, Input Registers, and Holding Registers.
6. **Fault Studio**: interactive tag overrides and scenario fault profiles.
7. **Event Log**: structured audit trail of physics events, client connections, and state transitions.

| Key | Action |
|---|---|
| `Tab` / `BackTab`, `1`–`7` | switch tabs |
| `↑` / `↓` | select a row or item |
| `Enter` / `F` | open the override modal for the selected tag |
| `Space` | toggle the selected coil or discrete input |
| `a` / `s` / `u` / `r` | acknowledge / shelve / unshelve / reset selected alarm (Alarms tab) |
| `/` | search tags by name (Process Tags tab) |
| `m` / `M` | filter tags by memory area (Process Tags tab) |
| `c` / `C` | clear the selected override / all overrides |
| `P` | pick a fault profile from the scenario |
| `?` | toggle keyboard shortcut help overlay |
| `Q` / `Esc` | quit and restore the terminal |

An override is pinned until you clear it, unless the fault profile that applied it set
`duration_secs`, in which case it lifts on its own. A write that the Process Image refuses is
logged as a failure; the log never reports a toggle that did not happen.

## Tests

185 tests: unit tests per crate, property tests for the register conversions, a doctest pinning
the byte-order table above, and integration tests that start the real servers on ephemeral ports.


```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo machete
```

CI runs those four on every push.

## Known limitations

Things that are deliberately missing, or that a user could reasonably trip over. This is the
honest list — anything not here is probably a bug rather than a design decision.

**Protocol coverage**

- Modbus implements standard supervisory and control function codes: FC01, FC02, FC03, FC04, FC05,
  FC06, FC15, FC16, FC22 (Mask Write Register), FC23 (Read/Write Multiple Registers), FC43/14
  (Read Device Identification), and FC08 (Diagnostics). File records (FC20/21) and FIFO calls
  answer `IllegalFunction` as they model storage media this simulator does not have.
- Modbus RTU is supported over serial interfaces; Modbus ASCII framing is not implemented.
- OPC-UA supports anonymous access or authenticated user credentials with `Basic256Sha256` and
  `Aes128Sha256RsaOaep` security policies. Custom certificates are stored under the configured `pki_dir`.
- OPC-UA variable writes synchronize back into the Process Image. Complex OPC-UA methods or custom
  alarm condition nodes are not mapped.

**Data model**

- `float64` has no layout in the Modbus specification to copy. WirePlant writes it as two 32-bit
  halves and only distinguishes "word swapped" from "not", so `big_endian` and `little_endian`
  produce the same four words. No bundled scenario uses it.
- Boolean tags are a threshold, not a type: any non-zero register value reads as true
  (`BOOL_THRESHOLD`, one constant, used everywhere).
- Register allocation is fixed when the scenario loads. There is no runtime remapping of a tag's
  address.

**Operations**

- Concurrency and throughput are validated under multi-client loads; soak testing across weeks of
  continuous drift has not been formally profiled.
- The dashboard renders at a fixed 50 ms frame rate regardless of the scenario tick rate.


## Maintainers & Company Ownership

WirePlant is created, maintained, and owned by **[macrodataware](https://macrodataware.com/)**.

- **Website**: [https://macrodataware.com/](https://macrodataware.com/)
- **GitHub**: [github.com/macrodataware](https://github.com/macrodataware)
- **Contact**: [engineers@macrodataware.com](mailto:engineers@macrodataware.com)

## License

Dual-licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Copyright (c) 2026 macrodataware. All rights reserved.
