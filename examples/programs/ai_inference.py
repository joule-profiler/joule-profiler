#!/usr/bin/env python3
"""CPU sentiment classification with Joule Profiler phase markers."""

import argparse
import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer

MODEL_ID = "distilbert/distilbert-base-uncased-finetuned-sst-2-english"

TEXTS = [
    "This movie was wonderful. I really enjoyed it.",
    "This movie was terrible. I would not recommend it.",
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repetitions",
        type=int,
        default=20,
        help="Number of inference batches (default: 20)",
    )
    args = parser.parse_args()
    if args.repetitions < 1:
        parser.error("--repetitions must be at least 1")

    # Fix the CPU thread count to make the workload easier to reproduce.
    torch.set_num_threads(1)

    print("__LOAD_MODEL__", flush=True)
    tokenizer = AutoTokenizer.from_pretrained(
        MODEL_ID, local_files_only=True
    )
    model = AutoModelForSequenceClassification.from_pretrained(
        MODEL_ID, local_files_only=True, use_safetensors=True
    ).to("cpu")
    model.eval()

    print("__PREPARE_INPUTS__", flush=True)
    inputs = tokenizer(
        TEXTS,
        padding=True,
        truncation=True,
        return_tensors="pt",
    )

    print("__INFERENCE__", flush=True)
    with torch.inference_mode():
        for _ in range(args.repetitions):
            logits = model(**inputs).logits

    print("__REPORT__", flush=True)
    predictions = logits.argmax(dim=-1).tolist()
    for text, prediction in zip(TEXTS, predictions):
        label = model.config.id2label[prediction]
        print(f"{label}: {text}")

    print(f"Processed {len(TEXTS) * args.repetitions} text predictions.")


if __name__ == "__main__":
    main()