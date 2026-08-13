---
title: 'Joule Profiler: A phase-based energy measurement tool'
filters:
  - pandoc-crossref
tags:
  - energy measurement
  - profiling
  - Intel RAPL
  - NVIDIA NVML
  - Linux
  - Rust
  - green computing
  - phase-based profiling
authors:
  - name: Jérémy Woirhaye
	orcid: 0009-0005-0777-9090
    equal-contrib: true
    affiliation: "1,2"
  - name: François Gibier
	orcid: 0009-0005-5164-8063
    equal-contrib: true 
    affiliation: "1,2"
  - name: Romain Rouvoy
	orcid: 0000-0003-1771-8791
    affiliation: "1,2"
affiliations:
 - name: Inria, France
   index: 1
 - name: University of Lille, France
   index: 2
date: 30 March 2026
bibliography: paper.bib
repository: https://github.com/joule-profiler/joule-profiler
---

# Summary

Joule Profiler is a lightweight Linux command-line tool for profiling a program’s energy consumption on bare-metal computing environments with minimal instrumentation overhead. It enables users to break a program's execution into user-defined phases (e.g., data loading, computation) and attribute energy use to each phase. The tool detects phase triggers from program output and automatically queries available hardware sources such as Intel RAPL (CPUs) or NVML (GPUs) to report system-wide energy consumption.

# Statement of need

Energy use in computing is a growing concern across research and industry. Software running in clouds, data centers, and edge devices contributes significantly to global energy consumption. Improving efficiency requires tools that measure energy during execution. Hardware counters on modern CPUs and GPUs (e.g., Intel RAPL) enable software-based energy measurement without external devices.

Researchers and developers need simple tools to measure the energy use of code segments without complex infrastructure. Joule Profiler addresses this with phase-based profiling that integrates easily into workflows.

# State of the field

Existing tools such as PowerAPI [@powerapi], Alumet [@alumet], Scaphandre [@scaphandre], and EnergiBridge [@sallou_energibridge_2024] monitor energy using these counters, often focusing on distributed and system-level observability. JouleIt [@jouleit], which inspired this work, demonstrated a light wrapper approach but lacked phase decomposition, GPU support, and modularity.

These solutions are better suited for system-level monitoring, not fine-grained analysis of program phases. Joule Profiler, in contrast, is designed for lightweight, single-invocation use, enabling energy attribution to specific program phases.

# Software design

## Phase-based profiling

\begin{figure}
	\centering
	\includegraphics[width=\linewidth]{images/phases.png}
	\caption{Process lifecycle illustrating sequential phases}
	\label{fig:phases}
\end{figure}

Traditional energy measurement provides either total energy or periodic power readings, leaving unclear which code regions are most energy-intensive. Joule Profiler enables phase-based profiling, letting users decompose execution into logical phases with minimal code changes by watching standard output for phase markers.

Joule Profiler scans standard output for user-defined patterns to detect phase boundaries. Developers can insert print statements at important program points if needed, enabling phase identification without intrusive instrumentation.

When a phase marker is detected, Joule Profiler records energy counter values at that boundary. After execution, it computes per-phase energy by subtracting these values.

## Software architecture

Joule Profiler has been designed to be modular. These modules collect energy and performance metrics from multiple hardware sources while keeping overhead low. To simplify extension and maintenance, the measurement logic is isolated from the hardware-specific implementations. Joule Profiler accesses RAPL counters via the `perf_event` interface [@linux_perf_event], which exposes hardware performance monitoring facilities. If `perf_event` is unavailable, the tool falls back to the powercap interface in Linux `sysfs` [@linux_powercap]. For NVIDIA GPUs, it uses the *NVIDIA Management Library* (NVML) [@nvidia_nvml] to retrieve power consumption on compatible hardware. Joule Profiler can also collect hardware performance counters via `perf_event`, allowing energy measurements to be correlated with performance events or split proportionally when multiple components contribute, as shown in Figure~\ref{fig:archi}.

\begin{figure}
    \centering
    \includegraphics[width=0.9\linewidth]{images/archi.png}
    \caption{Software architecture of Joule Profiler, showing the orchestrator coordinating measurement sources (RAPL, Nvidia-NVML, perf events) and exporting results to Terminal, JSON, and CSV formats.}
    \label{fig:archi}
\end{figure}

Internally, the tool is structured into layers. The core layer handles the main logic: detecting phases, aggregating metrics, and coordinating the measurement sources. Each source runs as an asynchronous task, enabling parallel data collection and maintaining temporal precision. The *Command-Line Interface* (CLI) layer manages user interaction, parses configuration options, and displays results. A source abstraction layer encapsulates each hardware backend, such as RAPL, NVML, or performance counters, in a separate module. This separation eases future integration of new sources without affecting the rest of the system. This design allows Joule Profiler to run on a large diversity of bare-metal machines based on Intel and AMD processors. While Joule Profiler can also be used in virtual environments, users are encouraged to check the availability of measurement sources.

# Software assessment

To validate its measurements, Joule Profiler was compared with the reference tools perf [@perfwiki] and Alumet [@alumet], both of which use RAPL counters but employ different strategies. This checks whether Joule Profiler introduces measurement bias.

Three scenarios were tested: (1) Parallel runs of Joule Profiler and perf (with CPU load) or Alumet (with GPU load) alongside a sleep command, ensuring identical hardware activity and measurement noise; (2) Sequential execution of Joule Profiler, perf, and Alumet with workload pinned to a single CPU core, to compare overhead and variability; and (3) A custom workload with periodic output tokens for testing phase detection precision.

Experiments used Grid’5000 nodes: Chirop (Intel Xeon, RAPL, 512 GiB RAM) and Chifflot (NVIDIA Tesla V100, NVML, 192 GiB RAM). Energy was measured from RAPL (PACKAGE, DRAM) and NVML (GPU). `perf_event` was used for access. Hyper-threading was disabled, and the CPU frequency governor was set to performance to reduce variability.

## Total energy comparison

### Parallel execution

We performed 4,000 measurements to achieve 80% power and applied a *Two One-Sided Tests* (TOST) procedure with an equivalence margin of 0.1% of the reference tool's mean to assess statistical equivalence.

\begin{figure}
	\centering
	\includegraphics[width=\linewidth]{images/full_comparison_parallel.pdf}
	\caption{\textit{Empirical Cumulative Distribution Function} (ECDF) of energy measurements (J) across RAPL domains (DRAM, PACKAGE) comparing perf and Joule Profiler, and GPU comparing Alumet and Joule Profiler.}
	\label{fig:rapl_energy_distribution}
\end{figure}

\begin{figure}
	\centering
	\includegraphics[width=\linewidth]{images/full_comparison_parallel2.pdf}
	\caption{Bland–Altman analysis of energy measurements (J) across RAPL domains (DRAM, PACKAGE) comparing perf and Joule Profiler, and GPU comparing Alumet and Joule Profiler.}
	\label{fig:rapl_bland_altman}
\end{figure}

\autoref{fig:rapl_energy_distribution} and \autoref{fig:rapl_bland_altman} show close agreement between Joule Profiler and the reference tools. For `DRAM-0`, the bias is 0.013 J with 96.8% of measurements within the _Limits of Agreement_ (LoA). For `PACKAGE-0`, the bias is 0.046 J with 95.8% within LoA, though variability increases at high energy values, consistent with known RAPL noise at the package level. For `GPU-0`, the bias is 1.39 J with 94.5% within LoA, reflecting the higher natural variability of GPU power sampling (coefficient of variation ~1.95% for both tools). The Pearson correlation between Joule Profiler and perf exceeded 99.9% for both RAPL domains and reached 99.5% against Alumet for GPU. The TOST null hypotheses of non-equivalence were rejected for all domains, confirming that Joule Profiler does not introduce a significant measurement bias.

### Sequential execution

A sequential execution (2,000 runs) was used to compare the tool's overhead and variability. All tools produced nearly identical distributions, with differences of <0.1% (RAPL) and <0.5% (GPU), indicating minimal overhead.

\begin{figure}
	\centering
	\includegraphics[width=0.9\linewidth]{images/full_comparison_sequential.pdf}
	\caption{Energy distribution (J) across RAPL domains (DRAM, PACKAGE) and GPU comparing perf, Alumet, and Joule Profiler.}
	\label{fig:sequential_comparison}
\end{figure}

\autoref{fig:sequential_comparison} presents the energy distributions of perf, Joule Profiler, and Alumet across sequential runs for RAPL domains and the GPU. In the parallel scenario, all tools report nearly identical values, with differences of less than 0.1% for RAPL domains and 0.5% for the GPU. The sequential execution results show that Joule Profiler does not introduce a significant overhead compared to Alumet and perf.

## Phase attribution precision

To evaluate the temporal accuracy of output-based phase detection, we used a custom program that printed periodic tokens at frequencies from 100 Hz to 1,000 Hz. We compared the print timestamp with the timestamp at which Joule Profiler detected each token. This was repeated 40 times at each frequency, with 10,000 measures per iteration, to achieve 80% statistical power.

\begin{figure}
	\centering
	\includegraphics[width=0.8\linewidth]{images/phase_delay_comparison.pdf}
	\caption{Median, first and last quartiles delay between phase detection time and real phase start.}
	\label{fig:phase_delay}
\end{figure}

\autoref{fig:phase_delay} shows that the baseline median detection delay is approximately 25 µs and remains stable across all frequencies. Joule Profiler introduces an additional median delay of 11 µs, with a coefficient of variation increasing from 23% below 800 Hz to 28% at 1,000 Hz. Under CPU load (stress-ng), the baseline delay drops to 2 µs, and Joule Profiler to 3 µs, confirming that idle-state latency is the primary source of delay. These results confirm that output-based instrumentation is viable for workloads with phase durations exceeding 1 ms, consistent with the RAPL counter's 1,000 Hz refresh rate.

# Research impact statement

Joule Profiler was initially developed at [Inria](https://www.inria.fr/fr) and the [University of Lille](https://www.univ-lille.fr) to benchmark _Function-as-a-Service_ (FaaS) workloads by isolating per-phase energy consumption and studying the impact of FaaS environment configurations. Since then, it has also been used as a reference tool for monitoring CI/CD workloads, providing detailed analysis of the energy consumption of build workflows. It is currently being extended to support distributed settings in the context of federated learning.

All validation experiments used the Grid'5000/SLICES-FR testbed, a shared French research infrastructure. Joule Profiler is intentionally compatible with its hardware and workflows. Joule Profiler is intended to be executed in bare-metal environments that expose the required sources (RAPL, NVML) under Linux.

Joule Profiler is open source (MIT) at [https://github.com/joule-profiler/joule-profiler](https://github.com/joule-profiler/joule-profiler), with versioned releases and documentation.

# AI usage disclosure

This submission used generative AI tools only during early project stages.

**Tool identification.**  
The authors used Claude Sonnet 4.5 (Anthropic) as a generative AI assistant during the project bootstrap phase.

**Scope of assistance.**  
AI assisted with repository structure, initial boilerplate, and early organizational guidance.

**Human verification and oversight.**  
All AI outputs were reviewed and validated by the authors, who made all key decisions and ensured compliance with standards.

The authors take full responsibility for the accuracy, originality, and integrity of the submitted work.

# Acknowledgements

This work received support from the France 2030 program under grant agreement `ANR-23-PECL-0003` ([CARECloud](https://carecloud.irisa.fr) project of the [PEPR CLOUD](https://pepr-cloud.fr/) research program), and from the Inria–Qarnot PULSE project ([https://defi-pulse.github.io/](https://defi-pulse.github.io/)). Experiments were carried out using the Grid'5000 testbed, supported by a scientific interest group hosted by Inria and including CNRS, RENATER, and several universities (see [https://www.grid5000.fr](https://www.grid5000.fr)).

# References
