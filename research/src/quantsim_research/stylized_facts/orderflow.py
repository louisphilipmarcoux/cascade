"""Order-flow stylized facts: trade-sign clustering and the Zumbach effect.

- Trade-sign autocorrelation: aggressor signs are strongly, slowly-decaying
  positively autocorrelated (order splitting / herding). A pure linear
  Hawkes flow reproduces *some* of this; long-memory needs more.
- Zumbach effect / time-reversal asymmetry: past *trends* predict future
  volatility more than past volatility predicts future trends. Linear Hawkes
  is time-reversal symmetric at the relevant order and is *expected to
  miss* this — the statistic quadratic Hawkes (Stage 2) exists to capture.
  Including it here is the honest, preregistered "what the v1 sim can't do".
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from quantsim_research.stylized_facts.autocorr import acf


def trade_sign_autocorr(signs: np.ndarray, max_lag: int = 500) -> np.ndarray:
    """ACF of the aggressor-sign series (±1) at trade-tick lags."""
    signs = np.asarray(signs, dtype=float)
    return acf(signs, min(max_lag, signs.size // 3))


@dataclass(frozen=True)
class ZumbachResult:
    # Correlation of past squared-trend with future realized volatility.
    trend_predicts_vol: float
    # Correlation of past volatility with future squared-trend.
    vol_predicts_trend: float
    # Asymmetry = trend→vol minus vol→trend. > 0 is the Zumbach effect.
    asymmetry: float


def zumbach_asymmetry(prices: np.ndarray, window: int = 50) -> ZumbachResult:
    """Measure the Zumbach time-reversal asymmetry from a price series.

    Compares, over rolling windows, the correlation between a past squared
    trend and future realized volatility against the time-reversed
    correlation. A positive asymmetry is the empirical Zumbach effect.
    """
    prices = np.asarray(prices, dtype=float)
    returns = np.diff(np.log(prices))
    n = returns.size
    if n < 3 * window:
        return ZumbachResult(float("nan"), float("nan"), float("nan"))

    # Rolling squared trend (past) and realized vol (future) at each split.
    trend_sq = []
    fut_vol = []
    past_vol = []
    fut_trend_sq = []
    for t in range(window, n - window):
        past = returns[t - window : t]
        future = returns[t : t + window]
        trend_sq.append(past.sum() ** 2)
        fut_vol.append(np.sum(future**2))
        past_vol.append(np.sum(past**2))
        fut_trend_sq.append(future.sum() ** 2)

    trend_sq = np.array(trend_sq)
    fut_vol = np.array(fut_vol)
    past_vol = np.array(past_vol)
    fut_trend_sq = np.array(fut_trend_sq)

    def corr(a: np.ndarray, b: np.ndarray) -> float:
        if a.std() == 0 or b.std() == 0:
            return 0.0
        return float(np.corrcoef(a, b)[0, 1])

    tv = corr(trend_sq, fut_vol)
    vt = corr(past_vol, fut_trend_sq)
    return ZumbachResult(trend_predicts_vol=tv, vol_predicts_trend=vt, asymmetry=tv - vt)


def imbalance_predictiveness(
    bid_qty: np.ndarray,
    ask_qty: np.ndarray,
    mid: np.ndarray,
) -> dict[str, float]:
    """Out-of-sample AUC of L1 order-book imbalance predicting the next mid
    move (logistic regression, chronological 70/30 split)."""
    from sklearn.linear_model import LogisticRegression
    from sklearn.metrics import roc_auc_score

    bid_qty = np.asarray(bid_qty, dtype=float)
    ask_qty = np.asarray(ask_qty, dtype=float)
    mid = np.asarray(mid, dtype=float)
    imbalance = (bid_qty - ask_qty) / (bid_qty + ask_qty + 1e-12)
    next_move = np.sign(np.diff(mid))
    # Align: imbalance[t] predicts sign of mid[t+1]-mid[t].
    x = imbalance[:-1]
    y = next_move
    keep = y != 0  # drop flat moves
    x = x[keep].reshape(-1, 1)
    y = (y[keep] > 0).astype(int)
    if x.shape[0] < 50 or len(np.unique(y)) < 2:
        return {"auc": float("nan"), "n": int(x.shape[0])}
    split = int(0.7 * x.shape[0])
    model = LogisticRegression()
    model.fit(x[:split], y[:split])
    proba = model.predict_proba(x[split:])[:, 1]
    if len(np.unique(y[split:])) < 2:
        return {"auc": float("nan"), "n": int(x.shape[0])}
    auc = float(roc_auc_score(y[split:], proba))
    return {"auc": auc, "n": int(x.shape[0]), "beta": float(model.coef_[0, 0])}
