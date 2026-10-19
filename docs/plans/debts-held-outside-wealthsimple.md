# Plan: debts held outside Wealthsimple: repayments in Cashflow, what is owed under Net asset value

## For the owner to decide

Nothing open. You settled the design on 2026-10-11 (brief 23) and the matching on 2026-10-19:
- **Cashflow:** repayments go in each month's bar beside margin interest, with no split between interest and principal.
- **Portfolio:** the Net asset value tile's second line shows `$x owed · $y net`, and there is no new tile.
- **Matching** (2026-10-19): a payment is matched to the outside account it went to, "you can find out where it went to and match it to the external account name". This replaces brief 23's "nothing is matched by name", which counted only Payments Canada's loan and mortgage codes and left out a card or line of credit paid by pre-authorized debit.

## Scope

Wealthsimple's net worth feature lists the person's outside accounts, linked through an aggregator or entered by hand, with their balances. Payments from Wealthsimple accounts to outside debts are rows in the activity feed the app already pulls. This plan:
- reads the outside accounts with each pull;
- matches each outgoing payment to the outside debt it paid;
- shows the month's repayments in the Cashflow bar;
- shows what is owed, and the net, under Net asset value.

Touched:
- the Wealthsimple pull: one query, `FetchIdentityNetWorthAccounts`;
- the book: a migration for the outside accounts and the person's answers;
- the engine's `CashMonth` and the Portfolio figures;
- the wire, and the Cashflow and Portfolio pages;
- `SPEC.md`: cashflow-11, cashflow-14, portfolio-5 and portfolio-7.

Out of scope:
- the interest part of an outside payment (your ruling);
- a debt-to-asset ratio, or a Net worth tile (withdrawn in brief 23);
- household scope, since only `OWN` is read;
- editing outside accounts from Bagholder.

## How the leading products do it

- **Copilot** treats a payment from a linked account to a manual card or loan as a transfer between two of the person's accounts. It asks the person to mark one when it can't tell (brief 23, `23/research-2.json`).
- **Monarch and Empower** show liabilities beside assets, with the net under the total. Kubera does the same ("assets minus debts"), and so does Wealthsimple's own Net worth page.

Each of them matches automatically where an identifier agrees. Where it doesn't, they ask the person once and apply the answer from then on. Nothing is left as a silent guess.

## Approach

### What is read, with each pull
- `FetchIdentityNetWorthAccounts` (`OWN`): each outside entity's id, type, name, institution, `accountNumberLast4`, balance and its as-of moment, currency, and whether it is linked or entered by hand. Kept the Type 2 way of brief 22: each statement is kept with its moment.
- The document is a selection of brief 23's `23/graphql/FetchIdentityNetWorthAccounts`, the fields matching needs. Each outside account's kind is read from its `__typename` (`StudentLoanExternalFinancialEntity` is `StudentLoan`), the bundle's own type names. Its reply shape is recorded from your own session and anonymised with `ws-anonymise`, as stage 3b did.
- The bill-pay payee list (`FetchFundingMethods`) is not read. A bill payment is matched like a debit, by its company's whole name against the debts' institutions, so the card-issuer flag adds nothing. That is one query fewer per sync.

### A liability
An outside entity whose type is one of: CreditCard, LineOfCredit, Heloc, Mortgage, AutoLoan, InstallmentLoan, PersonalLoan, StudentLoan, OtherLiability. The type set is tested as a set against the bundle's type map. The Wealthsimple credit card counts as a liability too: its balance is read today (`FetchCreditCardAccount`) and shown nowhere.

### Matching a payment to a liability
A payment is a withdrawal from a Wealthsimple account in scope that is a bill payment (`BILL_PAY`) or a pre-authorized debit (`AFT`). An EFT, an e-Transfer, and a payment to the Wealthsimple card stay what they are today.

The first rule that applies decides:
1. **The person's answer** for this payee, if one is kept.
2. **A bill payment:** its account's last four digits equal exactly one liability's `accountNumberLast4`.
3. **A bill payment** whose company's whole name equals the institution of exactly one liability (folded as in rule 4).
4. **A pre-authorized debit** whose stated purpose is a loan or mortgage, or names no purpose (brief 67, 3):
   - its originator's whole name equals the institution of exactly one liability;
   - names are compared once folded: case, accents, punctuation and legal-form words (Inc, Ltd, Ltée, Corp, LLC and the like). "Bank of Canada" is part of a name, so it isn't folded (brief 67, 2);
   - a name inside another never matches.
   The purposes are Wealthsimple's 130 `aftTransactionCategory` values (Standard 007), held as three sets: debt, no purpose, and every other purpose. A debit stating another purpose (insurance, a utility, a tax) is never a repayment by name.
5. **Otherwise it is unsettled.** That covers several candidates, a name no institution matches, or a loan or mortgage debit (Standard 007 codes 350–356 and 370–373) with no outside account entered.
   - An unsettled payment to an issuer of debt, or carrying a loan or mortgage code, is said in the Cashflow header once: "N payments could not be matched to a debt", with a button that asks the person which liability it paid, or "not a debt".
   - The answer is kept per payee: the originator's folded name, or the bill payee's company and last four digits. Later payments follow it.

No amount is matched, and no payee is named in code.

### Shown
- **Cashflow:** the month's repayments, in CAD on their days as the Interest charge rows are, go below the margin interest on the negative side of the bar, each in its own segment.
  - The legend gains Repayments.
  - The hover shows Repayments, and Net cashflow becomes distributions less interest less repayments.
  - The month count and the tiles are otherwise unchanged.
- **Portfolio:** the Net asset value tile keeps its value. Its subtitle, while any debt exists and the scope covers all your accounts, is `$x owed · $y net`. Under a narrower scope it stays as today, since an outside debt belongs to no account (brief 67, 1):
  - *owed* is the liabilities' latest balances in CAD;
  - *net* is the value less owed;
  - margin is inside the value already and is not counted again.
  With no debt, the subtitle stays as it is.

Hover shows each debt's balance with its as-of date, always (brief 67, 4). Wealthsimple itself says outside balances may not refresh by themselves.

## Acceptance criteria

- [ ] Generated cases for each matching rule:
  - a card bill payment by last four digits;
  - a bill payment to an issuer with one card;
  - a pre-authorized debit by institution name, with suffix and case variants;
  - two debts at one institution, which is unsettled and then settled by an answer;
  - a utility bill payment, an e-Transfer, an EFT and the Wealthsimple card's own payment, none of which is a repayment;
  - an answer applied to a later payment.
- [ ] Cashflow: a month with distributions, interest and repayments draws three segments, and its hover sums them. Engine and page tests, plus an e2e with a stood-in pull.
- [ ] Portfolio: the subtitle with debt, without debt, and under each scope (only the full scope shows it). The hover shows each as-of date. The Wealthsimple card's balance is counted, and margin is not counted twice.
- [ ] Reply shapes recorded from your session and anonymised. Each new query is read strictly (a named mismatch, never a quiet zero).
- [ ] The book migration with its schema snapshot and clear-all classification. `wire.ts` is regenerated.
- [ ] The capacity job is green. The one query per pull is read with the statements.
- [ ] Your own session, read only, says how many of your payments rules 2 to 4 settle and how many are left to ask. Counts only, never names.

## Verification

- **Matching** (`wealthsimple/src/repayments.rs`), generated cases:
  - by last four digits;
  - a bill payment, then a debit, by whole institution name, across case, accents and legal-form variants;
  - substring traps;
  - an insurance-coded debit from the lender (not counted) against a loan-coded one (counted);
  - two debts at one institution asked, then settled by an answer;
  - an answer to a removed debt asked again;
  - the three purpose sets checked against Wealthsimple's 130 purposes.
- **The reader** (`wealthsimple/src/read.rs`): the outside accounts as stated; a reply of another shape is a named mismatch.
- **The book and the server** (`engine_inputs.rs`, `each_payment_is_matched_to_the_outside_debt_it_paid`): the recorded bill payments and loan debit stored as a pull stores them and matched to made-up debts at their payees.
  - An expired bill payment repays nothing.
  - A chequing account is not a debt.
  - A second debt at a payee makes it a question, and the answer settles it.
- **The engine** (`engine/tests/debts.rs`, `changes.rs`):
  - a month's repayments, including one before the first distribution;
  - owed under every account and under no single account;
  - the debts change equals a fresh build and reports what moved.
- **Clear data:** the new tables are classified and cleared with their kinds.
- **The answer route:** refuses an empty payee, and a payee nobody was asked about.
- **The page** (`web/e2e/debts.spec.ts`):
  - the owed line, with its hover and as-of dates;
  - the line as before with nothing owed;
  - the Repayments segment, legend and hover;
  - the question and both answers sent.
  - Cashflow, Portfolio and menu specs: 42 passed. Page unit tests: 176 passed.
- **Not run here:** your own session. Its replies, and the count of payments rules 2 to 4 settle, need your Wealthsimple sign-in, which isn't on this server.

## Right to refuse

If `FetchIdentityNetWorthAccounts` doesn't answer through the session the app holds, stop and report before reaching for another sign-in.
