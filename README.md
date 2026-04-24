# Joule Profiler ⚡

[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Linux](https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)](https://www.linux.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg?style=for-the-badge)](LICENSE)
[![Documentation](https://img.shields.io/badge/docs-mdbook-blue?style=for-the-badge)](https://jwoirhaye.github.io/joule-profiler/)

A modular tool for measuring energy consumption and performance metrics of programs on Linux systems.

## Key Features

- **Multiple Metric Sources**: RAPL (powercap/perf), perf_event counters, NVIDIA GPU (NVML)
- **Phase-Based Profiling**: Measure energy consumption by program phases
- **Extensible Architecture**: Easy to add custom metric sources
- **Low Overhead**: Minimal impact on measured programs
- **Multiple Output Formats**: Terminal, JSON, CSV

## Quick Start

### Installation

```bash
# Quick install (recommended)
curl -fsSL https://raw.githubusercontent.com/jwoirhaye/joule-profiler/main/install.sh | bash

# Or build from source
git clone https://github.com/jwoirhaye/joule-profiler.git
cd joule-profiler
cargo build --release
sudo cp target/release/joule-profiler /usr/local/bin/
```

### Basic Usage

```bash
# Phase-based profiling
sudo joule-profiler phases -- python workload.py

# JSON output
sudo joule-profiler --json phases -- ./benchmark

# GPU profiling (NVIDIA)
sudo joule-profiler --gpu phases-- ./gpu-workload
```

## Documentation

**[Full Documentation](https://jwoirhaye.github.io/joule-profiler/)**

- [Quickstart](https://jwoirhaye.com/joule-profiler/quickstart.html)
- [Metric Sources](https://jwoirhaye.com/joule-profiler/sources/overview.html)
- [Examples](https://jwoirhaye.com/joule-profiler/examples/overview.html)

## What Makes Joule Profiler Different?

### Phase-Based Energy Profiling

Unlike traditional profilers, Joule Profiler can measure energy consumption of specific program phases, helping identify
which sections contribute most to energy usage.

```python
# example.py
print("__INIT__")
# initialization code
print("__COMPUTE__")
# heavy computation
print("__CLEANUP__")
```

```bash
sudo joule-profiler phases -- python example.py
```

### Multiple Metric Sources

| Source              | Metrics              | Requirements                  |
|---------------------|----------------------|-------------------------------|
| **RAPL** (powercap) | RAPL domains energy  | Intel CPU, kernel 3.13+       |
| **RAPL** (perf)     | RAPL domains energy  | Intel CPU, perf_event support |
| **perf_event**      | Performance counters | Linux perf support            |
| **NVML**            | GPU energy           | NVIDIA GPU                    |

## Platform Support

- **OS**: Linux (kernel 3.13+)
- **CPU**: Intel (RAPL)
- **GPU**: NVIDIA (NVML support)
- **Permissions**: Root or appropriate capabilities required

## Common Use Cases

- **Energy optimization**: Identify energy-intensive code sections
- **Performance analysis**: Correlate energy with performance counters
- **Green computing**: Measure and reduce carbon footprint
- **Benchmarking**: Compare energy efficiency across implementations

## Contributing

Contributions are welcome! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- [JouleIt](https://github.com/powerapi-ng/jouleit) by [@powerapi-ng](https://github.com/powerapi-ng) for inspiration

## Contact

- **Issues**: [GitHub Issues](https://github.com/jwoirhaye/joule-profiler/issues)
- **Discussions**: [GitHub Discussions](https://github.com/jwoirhaye/joule-profiler/discussions)
- **Author**: [@jwoirhaye](https://github.com/jwoirhaye), [@FrancoisGib](https://github.com/FrancoisGib)

---

**[Read the Full Documentation](https://jwoirhaye.github.io/joule-profiler/)** | *
*[⭐ Star on GitHub](https://github.com/jwoirhaye/joule-profiler)**
