# Wealthsimple's monthly statements

- `lira-2025-06.json`: the reply to `FetchMonthlyStatementWithTransactions` for the owner's LIRA, June 2025, as Wealthsimple sent it to its own Documents page (captured by the owner from the page's network tab, 2026-09-27; its ids already stood in for the real ones).
- `forms-brokerage.json`, `forms-coins.json`, `forms-cash.json`: every form of row the owner's statements write, captured 2026-10-26 from a read-only copy of the owner's book, every value made up on the owner's Mac. `forms-brokerage.json` is a brokerage statement with lists per currency, whose top-level list is not the lists joined (it is the CAD list without its last row); `forms-coins.json` a coin account's statement, with no lists per currency (`activitiesPerCurrency` null) and every `unit` null; `forms-cash.json` a cash statement, whose rows state no `unit`.

The one feed kind the same capture found and the recorded feed lacked, `SPEND`/`PREPAID`, was added to `../wealthsimple/edited-activity-one-per-kind.json`.

## The recorded forms

Every form of row the four statements above write, one line each: the statement's `__typename`, the row's `transactionType`, its description's form, and what its `unit` names. A form writes a date as `D`, a number as `N` (a leading `-` or `$` kept), the leading ticker as `SYM` wherever it recurs, the security's name between ` - ` and `:` as `NAME`, a contract's `CALL` or `PUT` as `RIGHT`, a coin's symbol after `Purchase of N ` or `Sale of N ` as `SYM`, and the free text after `Margin Interest Charges for ` as `NAME`; a symbol in any other place stays as recorded (made up). The unit style is `currency` (`$CAD`, `$USD`), `symbol`, `null`, or `absent` where the row has no `unit` (a cash statement's). `tests/forms.rs` holds the made-up broker (`src/generated.rs`) to state no row of a form not listed here.

```
typename | code | form | unit style
BrokerageMonthlyStatementObject | ASSIGN | SYM N USD RIGHT D: Assigned -N contract (executed at D) | null
BrokerageMonthlyStatementObject | ASSIGN | SYM N USD RIGHT D: Assigned -N contract (executed at D) | symbol
BrokerageMonthlyStatementObject | BUY | Purchase of N SYM (executed at D) | symbol
BrokerageMonthlyStatementObject | BUY | Purchase of N SYM (executed at D), FX Rate: N | currency
BrokerageMonthlyStatementObject | BUY | Purchase of N SYM (executed at D), FX Rate: N | null
BrokerageMonthlyStatementObject | BUY | Purchase of N SYM (executed at D), FX Rate: N, Fee charged $N | null
BrokerageMonthlyStatementObject | BUY | SYM - NAME: Bought N shares (executed at D) | currency
BrokerageMonthlyStatementObject | BUY | SYM - NAME: Bought N shares (executed at D) | null
BrokerageMonthlyStatementObject | BUY | SYM - NAME: Bought N shares at $N per share (executed at D) | currency
BrokerageMonthlyStatementObject | BUY | SYM - NAME: Bought N shares at $N per share (executed at D) | symbol
BrokerageMonthlyStatementObject | BUY | SYM - NAME: Bought N shares at $N per share (executed at D), FX Rate: N | currency
BrokerageMonthlyStatementObject | BUYTOCLOSE | SYM N USD RIGHT D: Bought N contract (executed at D) | currency
BrokerageMonthlyStatementObject | BUYTOCLOSE | SYM N USD RIGHT D: Bought N contract (executed at D), Fee: $N | currency
BrokerageMonthlyStatementObject | BUYTOCLOSE | SYM N USD RIGHT D: Bought N contract (executed at D), Fee: $N | null
BrokerageMonthlyStatementObject | BUYTOOPEN | SYM N USD RIGHT D: Bought N contract (executed at D) | currency
BrokerageMonthlyStatementObject | BUYTOOPEN | SYM N USD RIGHT D: Bought N contract (executed at D), Fee: $N | currency
BrokerageMonthlyStatementObject | BUYTOOPEN | SYM N USD RIGHT D: Bought N contract (executed at D), Fee: $N | null
BrokerageMonthlyStatementObject | CONT | Contribution (executed at D) | currency
BrokerageMonthlyStatementObject | CONT | Contribution (executed at D) | null
BrokerageMonthlyStatementObject | CONT | Deposit | currency
BrokerageMonthlyStatementObject | CORRECTION | Cash correction (executed at D) | null
BrokerageMonthlyStatementObject | CRYPTORWD | -N of XQB rewards earned | null
BrokerageMonthlyStatementObject | CRYPTORWD | N of XQC rewards earned | null
BrokerageMonthlyStatementObject | DIV | SYM - NAME: Cash dividend distribution, received at D | null
BrokerageMonthlyStatementObject | DIV | SYM - NAME: Cash dividend distribution, received on D, record date of D | currency
BrokerageMonthlyStatementObject | DIV | SYM - NAME: Cash dividend distribution, received on D, record date of D | null
BrokerageMonthlyStatementObject | DIV | SYM - NAME: Stock dividend distribution, received at D | null
BrokerageMonthlyStatementObject | EXPIRY | SYM N USD RIGHT D: Expired -N contract (executed at D) | null
BrokerageMonthlyStatementObject | EXPIRY | SYM N USD RIGHT D: Expired -N contract (executed at D) | symbol
BrokerageMonthlyStatementObject | EXPIRY | SYM N USD RIGHT D: Expired N contract (executed at D) | symbol
BrokerageMonthlyStatementObject | FEE | ACCOUNTING_REIMBURSEMENT (executed at D) | null
BrokerageMonthlyStatementObject | FEE | Fee charge of N in XQN - Examplecoin | null
BrokerageMonthlyStatementObject | FEE | Fee for purchase of N XQP (executed at D), FX Rate: N | currency
BrokerageMonthlyStatementObject | FEE | Fee for sale of N XQR (executed at D), FX Rate: N | currency
BrokerageMonthlyStatementObject | FEE | Fee paid on XQS-Examplecoin staking reward: -N | null
BrokerageMonthlyStatementObject | FEE | Fee paid on XQT-Examplecoin staking reward: N | null
BrokerageMonthlyStatementObject | FEE | Trading fee for purchase of N XQU (executed at D), FX Rate: N | currency
BrokerageMonthlyStatementObject | FEE | Trading fee for sale of N XQV (executed at D), FX Rate: N | currency
BrokerageMonthlyStatementObject | FEE | Withdrawal network fee charge of N in XQW - Examplecoin | symbol
BrokerageMonthlyStatementObject | FPLINT | Stock lending monthly interest payment | currency
BrokerageMonthlyStatementObject | FPLINT | Stock lending monthly interest payment | null
BrokerageMonthlyStatementObject | FPLINT | Stock lending monthly interest payment, FX Rate: N | currency
BrokerageMonthlyStatementObject | FPLINT | Stock lending monthly interest payment, FX Rate: N | null
BrokerageMonthlyStatementObject | FXCONVERSION | Convert CAD (executed at D) - $NUSD = $NCAD | currency
BrokerageMonthlyStatementObject | FXCONVERSION | Convert USD (executed at D) - $NUSD = $NCAD | currency
BrokerageMonthlyStatementObject | INTCHARGED | Margin Interest Charges for NAME | currency
BrokerageMonthlyStatementObject | LOAN | SYM - NAME: N Shares on loan (executed at D) | null
BrokerageMonthlyStatementObject | LOAN | SYM - NAME: N Shares on loan (executed at D) | symbol
BrokerageMonthlyStatementObject | NRT | Non-resident tax (executed at D) | currency
BrokerageMonthlyStatementObject | RECALL | SYM - NAME: Loan of N shares terminated (executed at D) | null
BrokerageMonthlyStatementObject | RECALL | SYM - NAME: Loan of N shares terminated (executed at D) | symbol
BrokerageMonthlyStatementObject | REFER | Referral bonus (D) | null
BrokerageMonthlyStatementObject | SELL | SYM - NAME: Sold N shares (executed at D) | currency
BrokerageMonthlyStatementObject | SELL | SYM - NAME: Sold N shares (executed at D) | null
BrokerageMonthlyStatementObject | SELL | SYM - NAME: Sold N shares (executed at D), FX Rate: N | null
BrokerageMonthlyStatementObject | SELL | SYM - NAME: Sold N shares at $N per share (executed at D) | currency
BrokerageMonthlyStatementObject | SELL | SYM - NAME: Sold N shares at $N per share (executed at D) | symbol
BrokerageMonthlyStatementObject | SELL | SYM - NAME: Sold N shares at $N per share (executed at D), FX Rate: N | currency
BrokerageMonthlyStatementObject | SELL | Sale of N SYM (executed at D) | symbol
BrokerageMonthlyStatementObject | SELL | Sale of N SYM (executed at D), FX Rate: N | currency
BrokerageMonthlyStatementObject | SELL | Sale of N SYM (executed at D), FX Rate: N | null
BrokerageMonthlyStatementObject | SELL | Sale of N SYM (executed at D), FX Rate: N, Fee charged $N | null
BrokerageMonthlyStatementObject | SELLTOCLOSE | SYM N USD RIGHT D: Sold N contract (executed at D) | currency
BrokerageMonthlyStatementObject | SELLTOCLOSE | SYM N USD RIGHT D: Sold N contract (executed at D), Fee: $N | currency
BrokerageMonthlyStatementObject | SELLTOCLOSE | SYM N USD RIGHT D: Sold N contract (executed at D), Fee: $N | null
BrokerageMonthlyStatementObject | SELLTOOPEN | SYM N USD RIGHT D: Sold N contract (executed at D) | currency
BrokerageMonthlyStatementObject | SELLTOOPEN | SYM N USD RIGHT D: Sold N contract (executed at D), Fee: $N | currency
BrokerageMonthlyStatementObject | SELLTOOPEN | SYM N USD RIGHT D: Sold N contract (executed at D), Fee: $N | null
BrokerageMonthlyStatementObject | STAKE | Staked N of XQF-Examplecoin | null
BrokerageMonthlyStatementObject | STKDIS | SYM - NAME: Distribution of -N shares (executed at D) | symbol
BrokerageMonthlyStatementObject | STKDIS | SYM - NAME: Distribution of N shares (executed at D) | null
BrokerageMonthlyStatementObject | STKDIS | SYM - NAME: Distribution of N shares (executed at D) | symbol
BrokerageMonthlyStatementObject | STKREORG | SYM - NAME: stock reorganization (executed at D) | symbol
BrokerageMonthlyStatementObject | TRFIN | Money transfer into the account (executed at D) | currency
BrokerageMonthlyStatementObject | TRFIN | Money transfer into the account (executed at D) | null
BrokerageMonthlyStatementObject | TRFIN | Money transfer into the account (executed at D), FX Rate: N | null
BrokerageMonthlyStatementObject | TRFIN | SYM - NAME: Transfer of N shares into the account (executed at D) | symbol
BrokerageMonthlyStatementObject | TRFINTF | Amalgamation transfer from Examplia Predecessorco: SYM - NAME: Transfer of N SYM into the account (executed at D) | null
BrokerageMonthlyStatementObject | TRFINTF | SYM - NAME: Transfer of N SYM into the account (executed at D) | null
BrokerageMonthlyStatementObject | TRFINTF | SYM - NAME: Transfer of N SYM into the account (executed at D) | symbol
BrokerageMonthlyStatementObject | TRFINTF | SYM - NAME: Transfer of N SYM into the account (executed at D), FX Rate: N | null
BrokerageMonthlyStatementObject | TRFINTF | Tax-free money transfer into the account (executed at D) | currency
BrokerageMonthlyStatementObject | TRFOUT | Money transfer out of the account (executed at D) | currency
BrokerageMonthlyStatementObject | TRFOUT | Money transfer out of the account (executed at D) | null
BrokerageMonthlyStatementObject | TRFOUT | Money transfer out of the account (executed at D), FX Rate: N | null
BrokerageMonthlyStatementObject | TRFOUT | SYM - NAME: Transfer of N shares out of the account (executed at D) | symbol
BrokerageMonthlyStatementObject | TRFOUTTF | SYM - NAME: Transfer of N SYM out of the account (executed at D) | null
BrokerageMonthlyStatementObject | TRFOUTTF | SYM - NAME: Transfer of N SYM out of the account (executed at D) | symbol
BrokerageMonthlyStatementObject | UNSTAKE | Unstaked N of XQC-Examplecoin | null
BrokerageMonthlyStatementObject | WD | Non-contribution withdrawal (executed at D) | currency
BrokerageMonthlyStatementObject | WD | Non-contribution withdrawal (executed at D) | null
BrokerageMonthlyStatementObject | WD | Withdrawal (executed at D) | currency
BrokerageMonthlyStatementObject | WHTFED | Federal withholding tax (executed at D) | currency
CashMonthlyStatementObject | AFT_IN | Direct deposit from Example Payee | absent
CashMonthlyStatementObject | AFT_OUT | Pre-authorized Debit to Example Payee | absent
CashMonthlyStatementObject | CASHBACK | Cash back - Wobblemart | absent
CashMonthlyStatementObject | EFT | Deposit | absent
CashMonthlyStatementObject | EFTOUT | Withdrawal | absent
CashMonthlyStatementObject | E_TRFIN | Interac e-Transfer® Received | absent
CashMonthlyStatementObject | E_TRFIN | Interac e-Transfer® Received from Example Payee | absent
CashMonthlyStatementObject | E_TRFOUT | Interac e-Transfer® Out | absent
CashMonthlyStatementObject | E_TRFOUT | Interac e-Transfer® Out to Example Payee | absent
CashMonthlyStatementObject | GIVEAWAY | Giveaway received | absent
CashMonthlyStatementObject | IFT | Deposit | absent
CashMonthlyStatementObject | INT | Interest earned | absent
CashMonthlyStatementObject | OBP_OUT | Online bill payment for Example Payee | absent
CashMonthlyStatementObject | OBP_OUT | Online bill payment for Example Payee, account ∙∙∙∙N (executed at D) | absent
CashMonthlyStatementObject | P2P_RECEIVED | Cash received | absent
CashMonthlyStatementObject | P2P_SENT | Cash sent | absent
CashMonthlyStatementObject | REIMB | Reimbursement received | absent
CashMonthlyStatementObject | TRFIN | Transfer in | absent
CashMonthlyStatementObject | TRFIN | Transfer in from Example Payee | absent
CashMonthlyStatementObject | TRFINTF | Transfer in | absent
CashMonthlyStatementObject | TRFINTF | Transfer in from Example Payee | absent
CashMonthlyStatementObject | TRFOUT | Transfer out | absent
CashMonthlyStatementObject | TRFOUT | Transfer out to Example Payee | absent
CashMonthlyStatementObject | TRFOUTTF | Transfer out | absent
CashMonthlyStatementObject | TRFOUTTF | Transfer out to Example Payee | absent
```
