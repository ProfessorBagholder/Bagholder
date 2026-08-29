# Bagholder

A local-first trading journal for Wealthsimple users. Runs on your computer, auto-syncs trades from Wealthsimple. Activity stays on your machine.

Provides a dashboard with total realized P&L, win rate/profit factor, expectancy, biggest winners/losers, annualized performance vs S&P500, equity curve, monthly P&L with some basic sorting/filtering.

## Run

```
python3 bagholder.py
```

The webapp opens on `http://127.0.0.1:8765`.

## Data

Login session and journal data are stored in `~/.bagholder/`.
