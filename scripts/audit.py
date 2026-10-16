#!/usr/bin/env python3
"""Independent accounting audit of saved working reports (no network requests)."""
import json
import math
from pathlib import Path

root = Path(__file__).resolve().parents[1]
market = json.loads((root / "data/market.json").read_text())
assert all("example.test" not in s["source"] for s in market["instruments"].values()), "Synthetic test data in working cache"
for strategy in (1, 2, 3):
    report = json.loads((root / f"data/strategy-{strategy}.json").read_text())
    for run in report["runs"]:
        name = f'{strategy} / {run["mode"]} / {run["label"]}'
        if run["status"] not in ("ready", "bankrupt"):
            print(f'{name}: UNAVAILABLE — {run["message"]}')
            continue
        assert run["points"] and run["points"][-1]["equity"] == run["final_equity"], name
        assert math.isclose(run["total_return"], (run["final_equity"] / run["initial"] - 1) * 100, abs_tol=1e-8), name
        peak = run["initial"]
        drawdown = 0
        for point in run["points"]:
            peak = max(peak, point["equity"])
            drawdown = max(drawdown, (1 - point["equity"] / peak) * 100)
        assert math.isclose(drawdown, run["max_drawdown"], abs_tol=1e-8), name
        assert all(s["trained_until"] < s["effective"] for s in run["selections"]), name
        if strategy in (1, 2):
            assert math.isclose(run["initial"] + sum(t["pnl"] for t in run["trades"]), run["final_equity"], abs_tol=1e-5), name
        else:
            for trade in run["trades"]:
                evidence = [e for e in market["dividends"] if e["ticker"] == trade["ticker"] and e["declared_on"] and e["last_buy_date"] and e["declared_on"] < trade["entry_date"] <= e["last_buy_date"] and (trade["exit_date"] is None or trade["exit_date"] <= e["last_buy_date"])]
                assert evidence, f"Trade without announcement evidence: {trade}"
        print(f'{name}: OK | {run["total_return"]:+.2f}% | {len(run["trades"])} trades | {run["start"]}…{run["end"]}')
