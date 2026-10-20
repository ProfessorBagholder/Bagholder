# Filings-model benchmark on air, 2026-10-20T16:40Z

- machine: Darwin arm64; Apple M3; 24 GB
- engine: llama.cpp b11750 (macos-arm64)
- devices:
    Available devices:
      MTL0: Apple M3 (18186 MiB, 18185 MiB free)
      BLAS: Accelerate (0 MiB, 0 MiB free)

## qwen3.5-0.8b-q4_0 (544M)
```
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| qwen35 0.8B Q4_0               | 526.50 MiB |   752.39 M | MTL,BLAS   |       4 |           pp768 |       1962.24 ± 0.68 |
| qwen35 0.8B Q4_0               | 526.50 MiB |   752.39 M | MTL,BLAS   |       4 |            tg48 |        108.68 ± 0.41 |
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| qwen35 0.8B Q4_0               | 526.50 MiB |   752.39 M | MTL,BLAS   |       4 |           pp768 |        702.23 ± 3.27 |
| qwen35 0.8B Q4_0               | 526.50 MiB |   752.39 M | MTL,BLAS   |       4 |            tg48 |         95.58 ± 0.31 |
```
| model | filing | time | tokens in/out | title | summary |
|---|---|---|---|---|---|
| qwen3.5-0.8b-q4_0 | 1000275-d91595dex991.txt | 1.1 s | 894/65 | Royal Bank of Canada Reports Third Quarter 2026 Earnings | Royal Bank of Canada announced record net income of $6.0 billion for the third quarter of 2026, up 11% year-over-year. |
| qwen3.5-0.8b-q4_0 | 1045810-nvda-20260902.txt | 0.9 s | 871/51 | NVIDIA Acquires Hugging Face | NVIDIA Corporation entered into a definitive agreement to acquire Hugging Face Inc. for an estimated $11.9 billion purchase price. |
| qwen3.5-0.8b-q4_0 | 1318605-tsla-20261002.txt | 1.0 s | 929/57 | Tesla 8-K Filing | Tesla, Inc. filed a Form 8-K disclosing its stock price of $2,000 per share on October 2, 2026. |
| qwen3.5-0.8b-q4_0 | 1594805-form6k-bfcmclosepressrelea.txt | 0.6 s | 382/40 | Annual Report | This document announces Shopify Inc.'s annual financial performance for the month of November 2024. |
| qwen3.5-0.8b-q4_0 | 2488-amd-20260926.txt | 0.8 s | 891/38 | Merger Agreement | Advanced Micro Devices announces the completion of a merger with World Labs to acquire its equity interests. |
| qwen3.5-0.8b-q4_0 | 311337-tm2627015d1_ex99-1.txt | 1.2 s | 760/74 | Suncor announces offshore asset divestiture and share repurchase increase | Suncor Energy announces a definitive agreement to sell 48% of its offshore assets to Ithaca Energy for $1.2 billion upfront and a contingent payment, while simultaneously increasing monthly share rep购 |
| qwen3.5-0.8b-q4_0 | 320193-a8-kex991q3202606272026.txt | 0.8 s | 724/44 | Apple Reports Strong Q3 Results | Apple announces third-quarter financial results with revenue up 16 percent and EPS up 29 percent. |
| qwen3.5-0.8b-q4_0 | 789019-d291965dex991.txt | 0.7 s | 661/38 | Segment Reporting Update | Microsoft announces updated segment reporting to reflect AI-driven business models and focus on agents and infrastructure. |

## qwen3.5-2b-q4_0 (1.1G)
```
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| qwen35 2B Q4_0                 |   1.12 GiB |     1.88 B | MTL,BLAS   |       4 |           pp768 |        885.64 ± 3.09 |
| qwen35 2B Q4_0                 |   1.12 GiB |     1.88 B | MTL,BLAS   |       4 |            tg48 |         60.15 ± 0.89 |
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| qwen35 2B Q4_0                 |   1.12 GiB |     1.88 B | MTL,BLAS   |       4 |           pp768 |        310.93 ± 4.24 |
| qwen35 2B Q4_0                 |   1.12 GiB |     1.88 B | MTL,BLAS   |       4 |            tg48 |         49.16 ± 0.07 |
```
| model | filing | time | tokens in/out | title | summary |
|---|---|---|---|---|---|
| qwen3.5-2b-q4_0 | 1000275-d91595dex991.txt | 2.5 s | 894/69 | Royal Bank of Canada Third Quarter 2026 Earnings Release | Royal Bank of Canada reported record net income of $6.0 billion and diluted EPS of $4.23 for the quarter ended July 31, 2026. |
| qwen3.5-2b-q4_0 | 1045810-nvda-20260902.txt | 1.8 s | 871/45 | NVIDIA Acquires Hugging Face | NVIDIA entered into a definitive agreement to acquire Hugging Face for approximately $11.9 billion. |
| qwen3.5-2b-q4_0 | 1318605-tsla-20261002.txt | 2.0 s | 929/53 | Tesla Inc. Press Release October 2, 2026 | Tesla Inc. published a press release regarding its financial results and operations as reported in this Form 8-K. |
| qwen3.5-2b-q4_0 | 1594805-form6k-bfcmclosepressrelea.txt | 1.4 s | 382/56 | Shopify Inc. Announces Record-High Sales During Black Friday and Cyber Monday | The company reports $11.5 billion in sales from merchants during the Black Friday-Cyber Monday period. |
| qwen3.5-2b-q4_0 | 2488-amd-20260926.txt | 1.7 s | 891/42 | Merger Agreement Announcement | AMD announces a merger to acquire all equity interests in World Labs Technologies for approximately $8.2 billion. |
| qwen3.5-2b-q4_0 | 311337-tm2627015d1_ex99-1.txt | 1.8 s | 760/57 | Suncor Agrees to Sell Offshore Assets | Suncor announced a definitive agreement to sell interests in Terra Nova, White Rose, and West White Rose offshore assets for $1.2 billion. |
| qwen3.5-2b-q4_0 | 320193-a8-kex991q3202606272026.txt | 1.8 s | 724/55 | Apple Reports Third Quarter Financial Results | Apple announced $109.4 billion revenue and $2.02 EPS for Q3 2026, with new June quarter records. |
| qwen3.5-2b-q4_0 | 789019-d291965dex991.txt | 1.5 s | 661/43 | Microsoft's FY27 Segment Structure Update | Microsoft is transitioning its reporting to two segments: Agents and Infra, and Devices and Consumer. |

