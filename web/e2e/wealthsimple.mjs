// Wealthsimple stood in on loopback for the signed-in browser tests (`serve.mjs
// --signed-in`): its search, answered from the listings below, and the token refresh
// a saved sign-in makes. Any other call is an error, said as Wealthsimple says one,
// so nothing else the app asks of it is made up here.

// Listings Wealthsimple's search knows that the made-up book does not hold: a second
// listing of a ticker the book holds on the TSX.
export const LISTINGS = [
  { id: 'sec-enb-nyse', symbol: 'ENB', name: 'Enbridge Inc.', primaryExchange: 'NYSE', primaryMic: 'XNYS', currency: 'USD', securityType: 'EQUITY' },
]

function result(l) {
  return { id: l.id, buyable: true, status: 'TRADING', currency: l.currency, securityType: l.securityType, wsTradeEligible: true, stock: { symbol: l.symbol, name: l.name, primaryExchange: l.primaryExchange, primaryMic: l.primaryMic } }
}

export function standIn(req, res) {
  let body = ''
  req.on('data', (c) => { body += c })
  req.on('end', () => {
    const send = (status, json) => {
      res.writeHead(status, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify(json))
    }
    if (req.url.startsWith('/oauth')) return send(200, { access_token: 'e2e-access', refresh_token: 'e2e-refresh', expires_in: 86400, identity_canonical_id: 'identity-e2e' })
    let asked = {}
    try { asked = JSON.parse(body || '{}') } catch { asked = {} }
    if (asked.operationName === 'FetchSecuritySearchResult') {
      const q = String(asked.variables?.query || '').toUpperCase()
      return send(200, { data: { securitySearch: { results: LISTINGS.filter((l) => l.symbol === q).map(result) } } })
    }
    send(200, { errors: [{ message: `${asked.operationName || 'this call'} is not stood in for the browser tests` }] })
  })
}
