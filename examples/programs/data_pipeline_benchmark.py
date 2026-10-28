#!/usr/bin/env python3
"""Reproducible data pipeline for profiling four application phases.

Usage:
    python3 examples/programs/data_pipeline_benchmark.py --iterations 1
    joule-profiler profile -- python3 examples/programs/data_pipeline_benchmark.py

The four pipeline stages are repeated deterministically. The result is
written to ``data_pipeline_result.json`` in the current directory by default.
Use ``--output`` to choose another path and ``--iterations`` to change the
workload size.
"""

import argparse
import csv
import json
from pathlib import Path


DATASET_PATH = Path(__file__).resolve().parents[1] / "data" / "pipeline_dataset.csv"


def positive_integer(value):
    iterations = int(value)
    if iterations < 1:
        raise argparse.ArgumentTypeError("iterations must be greater than zero")
    return iterations


def parse_arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("data_pipeline_result.json"),
        help="Output JSON path (default: ./data_pipeline_result.json)",
    )
    parser.add_argument(
        "--iterations",
        type=positive_integer,
        default=100,
        help="Repeat each pipeline stage (default: 100)",
    )
    return parser.parse_args()


def load_records():
    with DATASET_PATH.open(newline="", encoding="utf-8") as dataset_file:
        return list(csv.DictReader(dataset_file))


def transform_records(records):
    return [
        {
            "region": record["region"],
            "category": record["category"],
            "total_cents": int(record["quantity"]) * int(record["unit_price_cents"]),
        }
        for record in records
        if record["status"] == "completed"
    ]


def analyze_records(records):
    totals_by_region = {}
    totals_by_category = {}
    for record in records:
        region = record["region"]
        totals_by_region[region] = totals_by_region.get(region, 0) + record["total_cents"]
        category = record["category"]
        totals_by_category[category] = (
            totals_by_category.get(category, 0) + record["total_cents"]
        )

    return {
        "record_count": len(records),
        "total_amount_cents": sum(record["total_cents"] for record in records),
        "totals_by_category_cents": dict(sorted(totals_by_category.items())),
        "totals_by_region_cents": dict(sorted(totals_by_region.items())),
    }


def write_result(output_path, result, iterations):
    serialized_result = json.dumps(result, indent=2, sort_keys=True)
    for _ in range(1, iterations):
        serialized_result = json.dumps(result, indent=2, sort_keys=True)

    with output_path.open("x", encoding="utf-8") as output_file:
        output_file.write(serialized_result)
        output_file.write("\n")


def main():
    args = parse_arguments()

    print("__LOAD__", flush=True)
    try:
        records = load_records()
        for _ in range(1, args.iterations):
            records = load_records()
    except OSError as error:
        raise SystemExit(f"Unable to read dataset {DATASET_PATH}: {error}") from error

    print("__TRANSFORM__", flush=True)
    transformed_records = transform_records(records)
    for _ in range(1, args.iterations):
        transformed_records = transform_records(records)

    print("__ANALYZE__", flush=True)
    result = analyze_records(transformed_records)
    for _ in range(1, args.iterations):
        result = analyze_records(transformed_records)

    print("__WRITE__", flush=True)
    try:
        write_result(args.output, result, args.iterations)
    except OSError as error:
        raise SystemExit(f"Unable to write result {args.output}: {error}") from error


if __name__ == "__main__":
    main()
