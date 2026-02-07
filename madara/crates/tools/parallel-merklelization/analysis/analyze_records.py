#!/usr/bin/env python3
"""
Analyze `records.csv` produced by the parallel-merklelization POC bench aggregator.

This script focuses on explaining merkleization time in terms of:
- state diff "size" (touched contracts, unique storage keys, etc.)
- bonsai RocksDB wrapper ops (get/insert/remove/iter, overwrite rate, bytes)

Outputs:
- Printed regression summaries (simple + multivariate) split by kind=seq/par
  and also on the combined dataset with a kind indicator.
- Scatter plots with trend lines to visualize the relationship and mode split.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import pandas as pd
import seaborn as sns
from matplotlib import pyplot as plt


@dataclass
class FitResult:
    coef: dict[str, float]
    intercept: float
    r2: float
    n: int


def fit_ols(df: pd.DataFrame, y: str, xs: list[str]) -> FitResult:
    d = df.dropna(subset=[y] + xs).copy()
    n = len(d)
    if n == 0:
        return FitResult(coef={x: float("nan") for x in xs}, intercept=float("nan"), r2=float("nan"), n=0)

    X = d[xs].to_numpy(dtype=float)
    yv = d[y].to_numpy(dtype=float)
    X1 = np.column_stack([np.ones(len(X)), X])

    beta, *_ = np.linalg.lstsq(X1, yv, rcond=None)
    yhat = X1 @ beta
    ss_res = float(np.sum((yv - yhat) ** 2))
    ss_tot = float(np.sum((yv - float(np.mean(yv))) ** 2))
    r2 = 1.0 - (ss_res / ss_tot if ss_tot > 0 else 0.0)

    intercept = float(beta[0])
    coef = {x: float(beta[i + 1]) for i, x in enumerate(xs)}
    return FitResult(coef=coef, intercept=intercept, r2=r2, n=n)


def fmt_fit(name: str, fit: FitResult) -> str:
    parts = [f"{name}: n={fit.n} R^2={fit.r2:.3f} intercept={fit.intercept:.4g}"]
    for k, v in fit.coef.items():
        parts.append(f"  {k}={v:.4g}")
    return "\n".join(parts)


def add_derived_metrics(df: pd.DataFrame) -> pd.DataFrame:
    out = df.copy()
    out["kind"] = out["kind"].astype(str)
    out["is_par"] = (out["kind"] == "par").astype(int)

    out["get_per_trie_insert"] = np.where(
        out["bonsai_insert_trie_calls"] > 0,
        out["bonsai_get_calls_total"] / out["bonsai_insert_trie_calls"],
        np.nan,
    )
    out["iter_keys_per_trie_insert"] = np.where(
        out["bonsai_insert_trie_calls"] > 0,
        out["bonsai_iter_keys_total"] / out["bonsai_insert_trie_calls"],
        np.nan,
    )
    out["contains_per_trie_insert"] = np.where(
        out["bonsai_insert_trie_calls"] > 0,
        out["bonsai_contains_calls_total"] / out["bonsai_insert_trie_calls"],
        np.nan,
    )
    out["overwrite_per_trie_insert"] = np.where(
        out["bonsai_insert_trie_calls"] > 0,
        out["bonsai_insert_overwrite_total"] / out["bonsai_insert_trie_calls"],
        np.nan,
    )
    return out


def scatter_with_fit(df: pd.DataFrame, x: str, y: str, out_path: Path, title: str) -> None:
    plt.figure(figsize=(9, 6))
    sns.scatterplot(data=df, x=x, y=y, hue="kind", alpha=0.35, s=12)
    sns.regplot(data=df[df["kind"] == "seq"], x=x, y=y, scatter=False, color="#1f77b4", label="seq fit")
    sns.regplot(data=df[df["kind"] == "par"], x=x, y=y, scatter=False, color="#ff7f0e", label="par fit")
    plt.title(title)
    plt.tight_layout()
    out_path.parent.mkdir(parents=True, exist_ok=True)
    plt.savefig(out_path, dpi=160)
    plt.close()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--records-csv", type=Path, required=True)
    ap.add_argument("--out-dir", type=Path, required=True)
    args = ap.parse_args()

    df = pd.read_csv(args.records_csv)
    df = add_derived_metrics(df)

    out_dir = args.out_dir
    out_dir.mkdir(parents=True, exist_ok=True)

    # Fits the user expects: simple 1D + better multi-var model.
    for kind in ["seq", "par"]:
        d = df[df["kind"] == kind]
        print(fmt_fit(f"{kind}: merklization_ms ~ key_total", fit_ols(d, "merklization_ms", ["key_total"])))
        print()

        print(
            fmt_fit(
                f"{kind}: merklization_ms ~ unique_storage_keys + touched_contracts + class_updates_total + block_count",
                fit_ols(d, "merklization_ms", ["unique_storage_keys", "touched_contracts", "class_updates_total", "block_count"]),
            )
        )
        print()

        print(
            fmt_fit(
                f"{kind}: merklization_ms ~ trie_inserts + gets + iters + overwrite",
                fit_ols(
                    d,
                    "merklization_ms",
                    [
                        "bonsai_insert_trie_calls",
                        "bonsai_get_calls_total",
                        "bonsai_iter_keys_total",
                        "bonsai_insert_overwrite_total",
                    ],
                ),
            )
        )
        print()

    # Combined: check whether a 'kind' indicator is still significant after controlling for measured work.
    print(
        fmt_fit(
            "combined: merklization_ms ~ trie_inserts + gets + iters + overwrite + is_par",
            fit_ols(
                df,
                "merklization_ms",
                [
                    "bonsai_insert_trie_calls",
                    "bonsai_get_calls_total",
                    "bonsai_iter_keys_total",
                    "bonsai_insert_overwrite_total",
                    "is_par",
                ],
            ),
        )
    )
    print()

    # Plots
    scatter_with_fit(df, "key_total", "merklization_ms", out_dir / "key_total_vs_merkle_ms.png", "merklization_ms vs key_total")
    scatter_with_fit(
        df,
        "unique_storage_keys",
        "merklization_ms",
        out_dir / "unique_storage_keys_vs_merkle_ms.png",
        "merklization_ms vs unique_storage_keys",
    )
    scatter_with_fit(
        df,
        "touched_contracts",
        "merklization_ms",
        out_dir / "touched_contracts_vs_merkle_ms.png",
        "merklization_ms vs touched_contracts",
    )
    scatter_with_fit(
        df,
        "bonsai_insert_trie_calls",
        "merklization_ms",
        out_dir / "trie_inserts_vs_merkle_ms.png",
        "merklization_ms vs bonsai_insert_trie_calls",
    )
    scatter_with_fit(
        df,
        "bonsai_get_calls_total",
        "merklization_ms",
        out_dir / "gets_vs_merkle_ms.png",
        "merklization_ms vs bonsai_get_calls_total",
    )
    scatter_with_fit(
        df,
        "get_per_trie_insert",
        "merklization_ms",
        out_dir / "get_per_trie_insert_vs_merkle_ms.png",
        "merklization_ms vs get_per_trie_insert",
    )


if __name__ == "__main__":
    main()
