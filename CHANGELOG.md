# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-10-12

### Added
- **Core Engine & Physics Simulation**:
  - Continuous ODE solvers for process models (Coupled Tanks, First-Order Thermal Systems with analytical exponential solution, Industrial PID Controllers with anti-reset windup and low-pass filtered derivative on error or measurement).
  - Waveform generators: Sine, Triangle, Ramp, Constant, Trip Threshold, and Gaussian Random Walk via Box-Muller transform scaled by simulation delta time.
  - ANSI/ISA-18.2 compliant alarm state engine with qualification timers (on-delay/off-delay), deadband hysteresis, and full lifecycle transitions (Normal, Unacknowledged, Acknowledged, ReturnToNormalUnack, Latched, Shelved).
  - Embedded time-series SQLite Historian with Write-Ahead Logging (WAL) and automatic retention window pruning.
- **Industrial Protocol Servers**:
  - Modbus TCP & RTU server supporting FC01, FC02, FC03, FC04, FC05, FC06, FC15, FC16, FC22 (Mask Write), FC23 (Read/Write Multiple Registers with 121-register write limit), FC43/14 (Device Identification), and FC08 (Diagnostics Loopback).
  - OPC-UA server supporting anonymous and authenticated access, configurable PKI certificates, `Basic256Sha256` / `Aes128Sha256RsaOaep` encryption policies, and bi-directional write synchronization back to Process Image memory.
  - MQTT telemetry publisher and command subscriber with authentication and connection recovery.
- **REST API & Telemetry**:
  - Axum HTTP server with authentication middleware (`Authorization: Bearer` and `X-API-Key`).
  - Public `/health` endpoint reflecting real-time engine liveness and uptime.
  - Public `/metrics` endpoint formatted per Prometheus text exposition specification 0.0.4.
  - Endpoints for plant state inspection, override injection/clearing, alarm acknowledgement/reset/shelving, and historian queries.
- **Terminal Operations Dashboard**:
  - 7-tab Ratatui interactive TUI: Process Tags, Alarms, Control Loops, Network & Services, Memory Map, Fault Studio, and Event Log.
- **CLI & Validation**:
  - CLI binary with `--tui`, `--headless`, `validate`, and `export-history` commands.
  - Strict TOML scenario validation rejecting unknown keys, non-finite values, out-of-range bounds, and unresolvable tag cross-references.
