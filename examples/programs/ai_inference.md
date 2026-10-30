# Profiling CPU sentiment classification

## Overview

I created this example to explore Joule Profiler’s phase-based profiling on an artificial intelligence task: sentiment classification with a pretrained DistilBERT model running locally on CPU.

The script `examples/programs/ai_inference.py` classifies two English movie reviews as positive or negative. No training or cloud deployment.

I chose this example because it matches my experience with Python and AI while keeping the contribution small and easy to reproduce. My goal is to provide a Joule Profiler usage example without changing its internal operation.

**I tested the script under WSL 2 with the `procfs` source. This test provides memory and I/O metrics along with phase durations, without energy measurements.**

## Approach and verification

I separated model downloading from execution to exclude downloading from the profiled phases. I then added markers to distinguish loading, text preparation, and inference.

I first ran the script on its own to check the predictions, then with Joule Profiler to check phase detection and JSON file generation. During this test, the program exited with code `0` and performed the expected 40 predictions.

The RAPL interfaces I checked were not accessible in my WSL environment. I therefore used `procfs` to verify the integration. I did not measure or estimate energy from these results.

## Prerequisites and installation

The following commands must be run from the Joule Profiler repository root containing the script. I installed Joule Profiler separately by following its instructions.

Create and activate a Python environment:

```bash
python3 -m venv ~/.venvs/joule-ai
source ~/.venvs/joule-ai/bin/activate
python -m pip install torch==2.15.0 --index-url https://download.pytorch.org/whl/cpu
python -m pip install transformers==5.22.0
```

These versions were used for functional verification with Python 3.12.3. On Ubuntu, if environment creation fails because `ensurepip` is missing, install the `python3.12-venv` package for this Python version.

## Download the model before profiling

Run:

```bash
python - <<'PY'
from transformers import AutoModelForSequenceClassification, AutoTokenizer

model_id = "distilbert/distilbert-base-uncased-finetuned-sst-2-english"
AutoTokenizer.from_pretrained(model_id)
AutoModelForSequenceClassification.from_pretrained(
    model_id, use_safetensors=True
)
PY
```

The tokenizer converts texts into numerical inputs. The model contains the learned classification parameters.
You can consult the [model card](https://huggingface.co/distilbert/distilbert-base-uncased-finetuned-sst-2-english).

## Run without profiling

With the environment activated, from the repository root:

```bash
python examples/programs/ai_inference.py --repetitions 20
```

Each repetition processes both texts: 20 repetitions therefore correspond to 40 predictions. Only the final batch’s labels are displayed.
```

## Profile with procfs

```bash
joule-profiler --sources procfs --output-format json --output-file /tmp/joule-ai-procfs.json profile -- python examples/programs/ai_inference.py --repetitions 20
```

Display the JSON file:

```bash
python -m json.tool /tmp/joule-ai-procfs.json
```

| Start | End | Work performed |
|---|---|---|
| `START` | `__LOAD_MODEL__` | Python startup, imports, and initial setup |
| `__LOAD_MODEL__` | `__PREPARE_INPUTS__` | Load tokenizer and model; configure evaluation mode |
| `__PREPARE_INPUTS__` | `__INFERENCE__` | Convert texts into tensors |
| `__INFERENCE__` | `__REPORT__` | Run inference batches |
| `__REPORT__` | `END` | Decode labels, display results, and exit |

## Interpretation and limitations

- I verified this example with Joule Profiler 3.0.1 under Ubuntu 24.04 on WSL 2. The `procfs` source provides memory and I/O metrics, not energy.
- Since RAPL counters were not accessible in my environment, I did not perform energy measurements. These require a compatible Linux machine.
- The `proc_*` metrics concern the process, while the `global_*` metrics concern the observed Linux system.
- This example verifies profiling functionality. To compare performance, measurements must be repeated under the same conditions and their variability taken into account.

## AI assistance

I used ChatGPT to support me with this contribution: preparing my working environment, obtaining the necessary commands, and resolving some of the errors encountered. The tool also helped me understand the results. I executed the commands and performed the tests in my environment. This experience allowed me to discover phase-based profiling and understand the difference between resource metrics and energy measurements.
