// A holding on the page a trade opens: the position standing in for the trade
// (SPEC §Portfolio, "A holding"), with the figures a trade does not have beside it.
//
// Each field reads through to the position when it is shown, so the stand-in is made
// once per holding and a moved price reaches only the elements that show that figure
// (a new object on every tick would run the whole page again).

import type { Position, Trade } from './model'
import { waits } from './dec'

export function holdingAsTrade(p: Position): Trade {
  return {
    status: 'open',
    exitDate: null,
    fees: { gaps: [] },
    flags: [],
    holding: true,
    get id() { return p.id },
    get position() { return p.id },
    get symbol() { return p.symbol },
    get underlying() { return p.underlying },
    get name() { return p.name },
    get exchange() { return p.exchange },
    get kind() { return p.kind },
    get currency() { return p.currency },
    get account() { return p.account },
    get accountId() { return p.accountId },
    get instrument() { return p.instrument },
    get security() { return p.security },
    get side() { return p.short ? 'COVER' : 'SELL' },
    get qty() { return p.qty },
    get entry() { return p.avg },
    get exit() { return p.last },
    get entryDate() { return p.opened },
    get lastDate() { return p.opened },
    get holdDays() { return waits(p.held) ? 0 : p.held },
    get pnl() { return p.unreal },
    get pnlCad() { return p.unreal },
    get pnlPct() { return p.unrealPct },
    get grade() { return p.grade },
    get thesis() { return p.thesis },
    get tags() { return p.tags },
    get journal() { return p.trade },
    get avg() { return p.avg },
    get mv() { return p.mv },
    get cost() { return p.cost },
    get last() { return p.last },
    // the page shows the day's change as a percentage, as the market's rows state it
    get percentChange() { return p.percentChange == null ? null : p.percentChange * 100 },
    get held() { return p.held },
  }
}
