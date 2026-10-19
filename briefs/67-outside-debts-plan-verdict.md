# Brief 67: Verdict on plan #557 (debts held outside Wealthsimple): Go with four changes

## Verdict

**Go.** The plan builds brief 23 as the owner settled it on 2026-10-11, with the owner's correction of 2026-10-19: a payment is matched to the outside account it went to. Brief 23's "nothing is matched by name" is withdrawn.

The order of matching is the category's:
- the person's answer;
- an identifier that agrees: the last four digits, or the card-issuer flag;
- the institution's name;
- otherwise, ask once and keep the answer.

The fields it relies on are in the recovered documents: `accountNumberLast4` and `institutionName` are in `ExternalFinancialEntityFields` (brief 23, `23/graphql/FetchIdentityNetWorthAccounts.graphql`).

## Required changes

1. **Net is shown only when the scope holds every account.** Outside debts belong to the person, not to any account. Under a scope narrower than all of the person's accounts (one account, a group, the filters), subtracting every debt from that scope's value gives a "net" that means nothing.
   - The subtitle shows `$x owed · $y net` only when the scope covers all of the person's accounts, the Wealthsimple card included.
   - Under a narrower scope it stays as it is today.
   - SPEC portfolio-5 says so.
   *Test:* each scope of a generated book, with and without debt.
2. **The name rule matches whole names only, after folding legal forms.**
   - Fold only legal-form words (Inc, Incorporated, Ltd, Limited, Ltée, Corp, Corporation, Co, LLC, LLP, S.A., and the like), case, accents and punctuation. "Bank of Canada" is part of a name, never a suffix: "Royal Bank of Canada" must not fold to "Royal".
   - Then compare for equality, never by substring, and only where exactly one liability's institution is equal. Anything else is asked.
   *Test:* generated names with legal-form, case, accent and punctuation variants match. A name that is a substring of another, or two banks sharing a word, doesn't.
3. **A pre-authorized debit whose stated purpose isn't a debt is never a repayment by name.** Standard 007's code names the purpose: insurance, utilities, taxes, investments, payroll, and so on. A lender also collects insurance premiums and investment contributions under the same originator name. So rule 4 applies only when the debit's category is a loan or mortgage code (350–356, 370–373) or a code that doesn't name a purpose. A debit whose code names a non-debt purpose is not a repayment unless the person says so.
   - The two sets of codes are taken from the standard's own list, held as sets in code, and tested as sets.
   *Test:* a generated debit from a matching institution under an insurance code is not counted, and the same under a loan code is.
4. **No unexplained 30 days.** "A balance older than 30 days" is a threshold with no stated source. Show each outside balance's as-of date on hover always, as Wealthsimple's own page shows "Last updated". Drop the threshold.

## Not changed

Everything else stands:
- the person's answers kept per payee and asked once, from the Cashflow header;
- the Wealthsimple card counted and margin not counted twice;
- the migration, the strict reads and the anonymised reply shapes;
- the right to refuse;
- the read-only count from the owner's session, as counts only.

Build in one push.
