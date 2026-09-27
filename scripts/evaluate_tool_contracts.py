#!/usr/bin/env python3
"""Run the optional prediction-shape probe against the local contract corpus.

This runner is deliberately provider-neutral: it reads JSON predictions from a
file or stdin, performs no network calls, and never reads credentials. It
compares predicted tool/argument shape only; it does not validate JSON Schema,
invoke runtime dispatch, or replace the deterministic runtime corpus and live
parity gate.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


DEFAULT_CASES = Path(__file__).resolve().parents[1] / "tests/tool-contract-cases/cases.json"


def load_json(path: str) -> Any:
    if path == "-":
        return json.load(sys.stdin)
    return json.loads(Path(path).read_text(encoding="utf-8"))


def shape_matches(expected: Any, actual: Any) -> bool:
    """Compare fixture shape while treating `$...` strings as wildcards."""
    if isinstance(expected, str) and expected.startswith("$"):
        return True
    if isinstance(expected, dict):
        if not isinstance(actual, dict):
            return False
        return all(key in actual and shape_matches(value, actual[key]) for key, value in expected.items())
    if isinstance(expected, list):
        if not isinstance(actual, list) or len(actual) < len(expected):
            return False
        return all(shape_matches(item, actual[index]) for index, item in enumerate(expected))
    return expected == actual


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Prediction-shape probe: compare model tool/argument predictions with corpus fixtures."
    )
    parser.add_argument("--cases", default=str(DEFAULT_CASES), help="Path to cases.json")
    parser.add_argument(
        "--predictions",
        help="JSON array/object of predictions, or '-' for stdin; omit to print the prediction-shape corpus contract",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help=(
            "Exit 1 when a prediction is missing or its tool/argument shape mismatches; "
            "this is not strict JSON Schema or runtime validation"
        ),
    )
    args = parser.parse_args()

    cases = load_json(args.cases)
    if not isinstance(cases, list):
        raise SystemExit("cases must be a JSON array")
    if args.predictions is None:
        print(
            json.dumps(
                {
                    "probe": "prediction-shape",
                    "runtimeValidation": "not-performed",
                    "caseCount": len(cases),
                    "cases": cases,
                },
                indent=2,
                sort_keys=True,
            )
        )
        return 0

    raw_predictions = load_json(args.predictions)
    if isinstance(raw_predictions, list):
        predictions = {item.get("id"): item for item in raw_predictions if isinstance(item, dict)}
    elif isinstance(raw_predictions, dict):
        predictions = raw_predictions
    else:
        raise SystemExit("predictions must be a JSON array or object")

    results = []
    for case in cases:
        case_id = case["id"]
        prediction = predictions.get(case_id, {})
        expected_args = case.get("arguments", {})
        actual_args = prediction.get("arguments")
        tool_ok = prediction.get("tool") == case.get("tool")
        args_ok = shape_matches(expected_args, actual_args)
        results.append(
            {
                "id": case_id,
                "toolMatch": tool_ok,
                "argumentShapeMatch": args_ok,
                "passed": tool_ok and args_ok,
            }
        )

    passed = sum(result["passed"] for result in results)
    report = {
        "probe": "prediction-shape",
        "runtimeValidation": "not-performed",
        "caseCount": len(results),
        "passed": passed,
        "failed": len(results) - passed,
        "results": results,
    }
    print(json.dumps(report, indent=2, sort_keys=True))
    return 1 if args.strict and passed != len(results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
