# Product Specification

## Goal

Replace FanControl with a modern, secure, open-source alternative written in Rust while keeping the same level of power and usability.

## Must-have features (v1)

### Sensors & Controls
- Discover and list all available temperature sensors, fan speeds, and controllable fans
- Support for motherboard Super I/O, EC, GPU, CPU package, etc. via PawnIO + plugins
- Real-time reading of temperatures and fan RPMs
- Manual control of fan duty cycles (%)

### Fan Curves
- Create / edit / delete custom fan curves
- Multiple points (temperature → duty)
- Linear interpolation between points
- Assign a curve to one or more controls
- Hysteresis / response time settings

### Profiles
- Save / load complete configurations (curves + assignments)
- Quick switch between profiles
- Auto-apply last used profile on startup

### UI Requirements
- Clear overview of all sensors and controls
- Visual fan curve editor (drag points)
- Live graphs (temperature + RPM over time)
- System tray icon with quick controls
- Dark theme by default (light optional)

### Reliability
- Graceful degradation if PawnIO is not installed
- Clear error messages when hardware is not supported
- No silent failures

## Nice-to-have (v1.x / v2)

- Multi-sensor curves (e.g. max of CPU + GPU)
- External sensor sources (HWInfo, plugins)
- Scheduling / time-based profiles
- Remote monitoring (optional)
- Linux support

## Non-goals (for now)

- RGB control
- Overclocking
- Cross-platform parity in v1
