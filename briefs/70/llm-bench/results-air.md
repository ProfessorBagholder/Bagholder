# Filings-model benchmark on air, 2026-10-20T15:34Z

- machine: Darwin arm64; Apple M3; 24 GB
- engine: llama.cpp b11750 (macos-arm64)
- devices:
    Available devices:
      MTL0: Apple M3 (18186 MiB, 18185 MiB free)
      BLAS: Accelerate (0 MiB, 0 MiB free)

## gemma-2-2b-q4km (1.6G)
```
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| gemma2 2B Q4_K - Medium        |   1.59 GiB |     2.61 B | MTL,BLAS   |       4 |           pp768 |        625.35 ± 0.81 |
| gemma2 2B Q4_K - Medium        |   1.59 GiB |     2.61 B | MTL,BLAS   |       4 |            tg48 |         47.15 ± 0.21 |
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| gemma2 2B Q4_K - Medium        |   1.59 GiB |     2.61 B | MTL,BLAS   |       4 |           pp768 |        162.91 ± 2.57 |
| gemma2 2B Q4_K - Medium        |   1.59 GiB |     2.61 B | MTL,BLAS   |       4 |            tg48 |         34.58 ± 0.43 |
```
| model | filing | time | tokens in/out | title | summary |
|---|---|---|---|---|---|
| gemma-2-2b-q4km | 1000275-d91595dex991.txt | 3.2 s | 856/58 | Royal Bank of Canada Q3 2026 Earnings Report | Royal Bank of Canada reports record third-quarter 2026 earnings, driven by strong performance across its business units. |
| gemma-2-2b-q4km | 1045810-nvda-20260902.txt | 2.3 s | 864/40 | NVIDIA Acquisition of Hugging Face | This filing announces NVIDIA's acquisition of Hugging Face, Inc. |
| gemma-2-2b-q4km | 1318605-tsla-20261002.txt | 2.7 s | 923/51 | Tesla, Inc. Quarterly Earnings Report | This filing announces Tesla, Inc.'s financial results for the quarter ending October 2, 2026. |
| gemma-2-2b-q4km | 1594805-form6k-bfcmclosepressrelea.txt | 2.0 s | 380/59 | Shopify Inc. November 2024 Sales Report | This filing reports Shopify's sales performance for November 2024, including a record-breaking Black Friday-Cyber Monday period. |
| gemma-2-2b-q4km | 2488-amd-20260926.txt | 2.7 s | 885/52 | AMD Announces Acquisition of World Labs Technologies | Advanced Micro Devices, Inc. announced an agreement to acquire World Labs Technologies, Inc. for approximately $8.2 billion. |
| gemma-2-2b-q4km | 311337-tm2627015d1_ex99-1.txt | 2.4 s | 742/49 | Suncor Announces Asset Sale and Share Repurchases | Suncor Energy announces the sale of non-core offshore assets and increased share repurchases. |
| gemma-2-2b-q4km | 320193-a8-kex991q3202606272026.txt | 2.4 s | 732/49 | Apple Q3 2026 Financial Results | Apple announces its third quarter 2026 financial results, reporting record revenue and EPS. |
| gemma-2-2b-q4km | 789019-d291965dex991.txt | 2.4 s | 666/52 | Microsoft FY27 Segment Structure and Investor Metrics | This filing announces Microsoft's updated segment structure for FY27, reflecting changes in business operations and AI focus. |

## gemma-3-1b-q4km (785M)
```
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| gemma3 1B Q4_K - Medium        | 762.49 MiB |   999.89 M | MTL,BLAS   |       4 |           pp768 |       1696.56 ± 1.15 |
| gemma3 1B Q4_K - Medium        | 762.49 MiB |   999.89 M | MTL,BLAS   |       4 |            tg48 |         87.00 ± 0.08 |
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| gemma3 1B Q4_K - Medium        | 762.49 MiB |   999.89 M | MTL,BLAS   |       4 |           pp768 |        575.00 ± 4.88 |
| gemma3 1B Q4_K - Medium        | 762.49 MiB |   999.89 M | MTL,BLAS   |       4 |            tg48 |         74.14 ± 1.04 |
```
| model | filing | time | tokens in/out | title | summary |
|---|---|---|---|---|---|
| gemma-3-1b-q4km | 1000275-d91595dex991.txt | 1.8 s | 862/75 | Royal Bank of Canada Q3 2026 Earnings Report | Royal Bank of Canada reported a record $6.0 billion net income for Q3 2026, up 11% YoY, driven by higher revenue and a strong ROE. |
| gemma-3-1b-q4km | 1045810-nvda-20260902.txt | 1.2 s | 859/55 | NVIDIA Acquisition of Hugging Face | NVIDIA announced a definitive agreement to acquire Hugging Face, a platform for open-source models, expanding its capabilities and resources. |
| gemma-3-1b-q4km | 1318605-tsla-20261002.txt | 1.3 s | 919/63 | Tesla, Inc. Form 8-K Filing – October 2, 2026 | Tesla, Inc. announces the filing of Form 8-K, detailing its financial results and registration. |
| gemma-3-1b-q4km | 1594805-form6k-bfcmclosepressrelea.txt | 0.8 s | 378/50 | Shopify Annual Report – November 2024 | This report details Shopify Inc.’s financial performance for November 2024. |
| gemma-3-1b-q4km | 2488-amd-20260926.txt | 1.3 s | 881/59 | AMD Filing – Advanced Micro Devices, Inc. Merger Agreement | Advanced Micro Devices, Inc. finalized a merger agreement with World Labs Technologies, Inc., involving a $8.2 billion acquisition. |
| gemma-3-1b-q4km | 311337-tm2627015d1_ex99-1.txt | 1.3 s | 752/70 | Suncor Divestment Agreement – Terra Nova, White Rose, West White Rose | Suncor will sell its offshore assets, including Terra Nova, to Ithaca Energy for $1.2 billion, aligning with long-term shareholder returns. |
| gemma-3-1b-q4km | 320193-a8-kex991q3202606272026.txt | 1.2 s | 732/60 | Apple Q3 2026 Financial Results | Apple announced a record quarter with $109.4 billion in revenue and a 16% year-over-year increase. |
| gemma-3-1b-q4km | 789019-d291965dex991.txt | 1.0 s | 665/48 | Microsoft Updates Segment Structure and Reporting | Microsoft is transitioning to two segments: Agents and Infra, with Azure focusing on consumption and improved security. |

## qwen3.5-0.8b-q4_0 (544M)
```
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| qwen35 0.8B Q4_0               | 526.50 MiB |   752.39 M | MTL,BLAS   |       4 |           pp768 |       1963.95 ± 2.00 |
| qwen35 0.8B Q4_0               | 526.50 MiB |   752.39 M | MTL,BLAS   |       4 |            tg48 |        107.35 ± 0.98 |
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| qwen35 0.8B Q4_0               | 526.50 MiB |   752.39 M | MTL,BLAS   |       4 |           pp768 |       656.18 ± 12.76 |
| qwen35 0.8B Q4_0               | 526.50 MiB |   752.39 M | MTL,BLAS   |       4 |            tg48 |         95.03 ± 0.26 |
```
| model | filing | time | tokens in/out | title | summary |
|---|---|---|---|---|---|
| qwen3.5-0.8b-q4_0 | 1000275-d91595dex991.txt | 1.7 s | 892/120 | (not JSON) |  |
| qwen3.5-0.8b-q4_0 | 1045810-nvda-20260902.txt | 1.7 s | 869/120 | (not JSON) |  |
| qwen3.5-0.8b-q4_0 | 1318605-tsla-20261002.txt | 1.6 s | 927/120 | (not JSON) |  |
| qwen3.5-0.8b-q4_0 | 1594805-form6k-bfcmclosepressrelea.txt | 1.3 s | 380/120 | (not JSON) |  |
| qwen3.5-0.8b-q4_0 | 2488-amd-20260926.txt | 1.6 s | 889/120 | (not JSON) |  |
| qwen3.5-0.8b-q4_0 | 311337-tm2627015d1_ex99-1.txt | 1.5 s | 758/120 | (not JSON) |  |
| qwen3.5-0.8b-q4_0 | 320193-a8-kex991q3202606272026.txt | 1.5 s | 722/120 | (not JSON) |  |
| qwen3.5-0.8b-q4_0 | 789019-d291965dex991.txt | 1.5 s | 659/120 | (not JSON) |  |

## lfm2.5-1.2b-q4_0 (673M)
```
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| lfm2 1.2B Q4_0                 | 661.25 MiB |     1.17 B | MTL,BLAS   |       4 |           pp768 |       1351.07 ± 0.74 |
| lfm2 1.2B Q4_0                 | 661.25 MiB |     1.17 B | MTL,BLAS   |       4 |            tg48 |        115.83 ± 0.36 |
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| lfm2 1.2B Q4_0                 | 661.25 MiB |     1.17 B | MTL,BLAS   |       4 |           pp768 |        538.98 ± 6.13 |
| lfm2 1.2B Q4_0                 | 661.25 MiB |     1.17 B | MTL,BLAS   |       4 |            tg48 |         93.95 ± 0.12 |
```
| model | filing | time | tokens in/out | title | summary |
|---|---|---|---|---|---|
| lfm2.5-1.2b-q4_0 | 1000275-d91595dex991.txt | 1.0 s | 878/33 | Q3 2026 Earnings Release | RBC reports record net income and diluted EPS. |
| lfm2.5-1.2b-q4_0 | 1045810-nvda-20260902.txt | 1.0 s | 836/38 | NVIDIA Acquires Hugging Face | NVIDIA acquires Hugging Face for $11.9 billion |
| lfm2.5-1.2b-q4_0 | 1318605-tsla-20261002.txt | 1.0 s | 888/38 | Tesla 8-K Filing Announcement | Tesla's 2026 Form 8-K disclosure |
| lfm2.5-1.2b-q4_0 | 1594805-form6k-bfcmclosepressrelea.txt | 0.6 s | 382/31 | Shopify Annual Report Submission | Announces Shopify's record sales performance. |
| lfm2.5-1.2b-q4_0 | 2488-amd-20260926.txt | 0.9 s | 851/32 | Merger Announcement | Announces acquisition of World Labs by Advanced Micro Devices. |
| lfm2.5-1.2b-q4_0 | 311337-tm2627015d1_ex99-1.txt | 0.9 s | 742/33 | Suncor Announces Asset Sale | Suncor sells offshore assets for shareholder returns |
| lfm2.5-1.2b-q4_0 | 320193-a8-kex991q3202606272026.txt | 0.8 s | 711/32 | Q3 2026 Financial Results Announcement | Apple reports record revenue and EPS. |
| lfm2.5-1.2b-q4_0 | 789019-d291965dex991.txt | 0.8 s | 669/30 | FY27 Segment Structure Update | Announces segment transition and reporting changes. |

## gemma-3-270m-q4_0 (240M)
```
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| gemma3 270M Q4_0               | 224.16 MiB |   268.10 M | MTL,BLAS   |       4 |           pp768 |      9434.97 ± 84.49 |
| gemma3 270M Q4_0               | 224.16 MiB |   268.10 M | MTL,BLAS   |       4 |            tg48 |        251.27 ± 0.70 |
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| gemma3 270M Q4_0               | 224.16 MiB |   268.10 M | MTL,BLAS   |       4 |           pp768 |     2660.84 ± 870.00 |
| gemma3 270M Q4_0               | 224.16 MiB |   268.10 M | MTL,BLAS   |       4 |            tg48 |        249.40 ± 0.37 |
```
| model | filing | time | tokens in/out | title | summary |
|---|---|---|---|---|---|
| gemma-3-270m-q4_0 | 1000275-d91595dex991.txt | 0.6 s | 862/83 | Royal Bank of Canada's Q3 2026 Earnings Report | Royal Bank of Canada (RBC) announced its Q3 2026 earnings, showing a strong performance across all financial statements, including net income, diluted EPS, and ROE. The report detailed the results of  |
| gemma-3-270m-q4_0 | 1045810-nvda-20260902.txt | 0.6 s | 859/95 | NVDA-20260902 0001045810 | NVDA-20260902 0001045810 is a United States securities and financial regulation company that is a leading technology company in the financial services sector. It is a publicly traded company, and its  |
| gemma-3-270m-q4_0 | 1318605-tsla-20261002.txt | 0.7 s | 919/112 | FILING TEXT | TSLA-20261002 FALSE 0001318605 0001318605 2026-10-02 UNITED STATES SECURITIES AND EXCHANGE COMMISSION WASHINGTON, DC 20549 FORM 8-K CURRENT REPORT REPORT. Pursuant to Section 13 or 15(d) of the SEC's  |
| gemma-3-270m-q4_0 | 1594805-form6k-bfcmclosepressrelea.txt | 0.5 s | 378/97 | FILING TEXT | 6-K 1 form6k-bfcmclosepressrelea.htm 6-K Document SECURITIES AND EXCHANGE COMMISSION Washington, D.C. 20549 FORM 6-K Report of Foreign Private Issuer Pursuant to Rule 13a-16 or 15d-16 of the SEC Act 1 |
| gemma-3-270m-q4_0 | 2488-amd-20260926.txt | 0.6 s | 881/107 | FILING TEXT | AMD-20260926 0000002488 false 0000002488 2026-09-26 UNITED STATES SECURITIES AND EXCHANGE COMMISSION Washington, D.C. 20549 FORM 8-K CURRENT REPORT REPORT ON ADVANCED MICRODECEPTION INC. (Exact name). |
| gemma-3-270m-q4_0 | 311337-tm2627015d1_ex99-1.txt | 0.5 s | 752/88 | FILING TEXT | EX-99.1 2 tm2627015d1_ex99-1.htm EXHIBIT 99.1 Exhibit 99.1 Suncor to divest non-core offshore assets and increase shareholder returns Unless otherwise noted, all financial figures are presented in CAD |
| gemma-3-270m-q4_0 | 320193-a8-kex991q3202606272026.txt | 0.5 s | 732/78 | FILING TEXT | The Company announced financial results for its fiscal 2026 third quarter ended June 27, 2026. The Company posted quarterly revenue of $109.4 billion, up 16 percent year over year. The Company gross,  |
| gemma-3-270m-q4_0 | 789019-d291965dex991.txt | 0.4 s | 665/56 | FILING TEXT | This presentation contains forward-looking statements, which are any predictions, projections, or other statements about future events. These statements are based on current expectations and assumes a |

## qwen3.5-2b-q4_0 (1.1G)
```
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| qwen35 2B Q4_0                 |   1.12 GiB |     1.88 B | MTL,BLAS   |       4 |           pp768 |        892.94 ± 0.18 |
| qwen35 2B Q4_0                 |   1.12 GiB |     1.88 B | MTL,BLAS   |       4 |            tg48 |         59.63 ± 0.17 |
| model                          |       size |     params | backend    | threads |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | ------: | --------------: | -------------------: |
| qwen35 2B Q4_0                 |   1.12 GiB |     1.88 B | MTL,BLAS   |       4 |           pp768 |        306.03 ± 4.25 |
| qwen35 2B Q4_0                 |   1.12 GiB |     1.88 B | MTL,BLAS   |       4 |            tg48 |         48.24 ± 1.12 |
```
| model | filing | time | tokens in/out | title | summary |
|---|---|---|---|---|---|
| qwen3.5-2b-q4_0 | 1000275-d91595dex991.txt | 3.2 s | 892/120 | (not JSON) |  |
| qwen3.5-2b-q4_0 | 1045810-nvda-20260902.txt | 3.1 s | 869/120 | (not JSON) |  |
| qwen3.5-2b-q4_0 | 1318605-tsla-20261002.txt | 3.1 s | 927/120 | (not JSON) |  |
| qwen3.5-2b-q4_0 | 1594805-form6k-bfcmclosepressrelea.txt | 2.6 s | 380/120 | (not JSON) |  |
| qwen3.5-2b-q4_0 | 2488-amd-20260926.txt | 3.1 s | 889/120 | (not JSON) |  |
| qwen3.5-2b-q4_0 | 311337-tm2627015d1_ex99-1.txt | 2.9 s | 758/120 | (not JSON) |  |
| qwen3.5-2b-q4_0 | 320193-a8-kex991q3202606272026.txt | 2.9 s | 722/120 | (not JSON) |  |
| qwen3.5-2b-q4_0 | 789019-d291965dex991.txt | 2.8 s | 659/120 | (not JSON) |  |

