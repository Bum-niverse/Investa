import { invoke } from "@tauri-apps/api/core";
import { type FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { FuturesPaperPanel } from "./FuturesPaperPanel";
import { INDICATOR_DEFINITIONS } from "./chartIndicators";
import { InteractiveCandleChart, type ChartSnapshot } from "./InteractiveCandleChart";
import { SymbolSearchInput } from "./SymbolSearchInput";

export type PaperAccountSnapshot = {
  mode: string;
  liveOrderEnabled: boolean;
  initialCashMinor: number;
  account: {
    accountId: string;
    currency: string;
    cashMinor: number;
    realizedPnlMinor: number;
    positions: Record<string, { symbol: string; quantity: number; quantityScale: number; costBasisMinor: number }>;
    eventCount: number;
    lastEventAtMs: number;
  };
  warning: string;
};

type Market = "kr" | "us" | "coin";
type Side = "buy" | "sell";
type Interval = "1d" | "1m";
type MarketQuote = { provider: string; symbol: string; currency: string; lastPriceMinor: number; observedAtMs: number };
type AccountsSnapshot = { accounts: PaperAccountSnapshot[]; liveOrderEnabled: boolean };
type ManualOrder = {
  orderId: string;
  market: Market;
  symbol: string;
  currency: string;
  side: Side;
  orderType: "limit";
  quantity: number;
  quantityScale: number;
  limitPriceMinor: number;
  status: "pending" | "cancelled";
  createdAtMs: number;
  updatedAtMs: number;
};
type KisStatus = {
  configured: boolean;
  connected: boolean;
  maskedAccountNumber: string | null;
  productCode: string | null;
  liveOrderEnabled: boolean;
  paperOrderEnabled: boolean;
  message: string;
};
type Costs = { buyFeeBps: number; sellFeeBps: number; sellTaxBps: number; slippageBps: number };
type CostPreset = { costs: Costs; source: string; basis: string };

const MARKET_INFO: Record<Market, { label: string; defaultSymbol: string; currency: "KRW" | "USD"; hint: string }> = {
  kr: { label: "국장", defaultSymbol: "005930", currency: "KRW", hint: "종목 코드 예: 005930" },
  us: { label: "미장", defaultSymbol: "AAPL", currency: "USD", hint: "티커 예: AAPL" },
  coin: { label: "코인", defaultSymbol: "KRW-XRP", currency: "KRW", hint: "업비트 원화 마켓 예: KRW-BTC" },
};
const SYMBOL_ALIASES: Record<string, string> = {
  삼성전자: "005930", 하이닉스: "000660", SK하이닉스: "000660", 한화에어로스페이스: "012450", 애플: "AAPL", 테슬라: "TSLA",
  비트코인: "KRW-BTC", 이더리움: "KRW-ETH", 리플: "KRW-XRP",
};
export const MARKET_COST_PRESETS: Record<Market, CostPreset> = {
  kr: {
    costs: { buyFeeBps: 1.5, sellFeeBps: 1.5, sellTaxBps: 20, slippageBps: 0 },
    source: "토스증권 Open API · 한국투자증권 매매관련 세금",
    basis: "KRX 수수료 0.015%, KOSPI·KOSDAQ 매도세 0.20%. NXT 체결 수수료는 0.014%입니다.",
  },
  us: {
    costs: { buyFeeBps: 10, sellFeeBps: 10, sellTaxBps: 0.206, slippageBps: 0 },
    source: "토스증권 Open API · 한국투자증권 해외주식 제비용",
    basis: "미국주식 수수료 0.10%, 매도 SEC Fee 0.00206%. 10달러 이하 토스 주문의 수수료 면제는 체결금액 확인이 필요합니다.",
  },
  coin: {
    costs: { buyFeeBps: 5, sellFeeBps: 5, sellTaxBps: 0, slippageBps: 0 },
    source: "Upbit 공식 KRW 마켓룰",
    basis: "KRW 마켓 거래 수수료 0.05%. 거래세 기본값은 없으며 이벤트·주문 방식에 따른 실제 요율은 체결 전 확인합니다.",
  },
};
const isTauriRuntime = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const resolveSymbol = (value: string) => SYMBOL_ALIASES[value.trim()] ?? value.trim().toUpperCase();
const createOrderId = (prefix: string) => `${prefix}-${Date.now()}-${crypto.randomUUID()}`;
const currencyDecimals = (currency: string) => currency === "KRW" ? 0 : 2;
const toMinor = (value: string, currency: string) => {
  const numeric = Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) return null;
  return Math.round(numeric * 10 ** currencyDecimals(currency));
};
const formatMoney = (minor: number, currency: string) => new Intl.NumberFormat("ko-KR", {
  style: "currency", currency, maximumFractionDigits: currencyDecimals(currency),
}).format(minor / 10 ** currencyDecimals(currency));

function NumericStepper({ value, onChange, min, max, step, ariaLabel }: { value: string; onChange: (value: string) => void; min: number; max?: number; step: number; ariaLabel: string }) {
  const decimals = Math.max(0, (String(step).split(".")[1] ?? "").length);
  const adjust = (direction: 1 | -1) => {
    const current = Number(value);
    const base = Number.isFinite(current) ? current : min;
    const next = Math.min(max ?? Number.MAX_SAFE_INTEGER, Math.max(min, base + direction * step));
    onChange(decimals ? next.toFixed(decimals).replace(/\.?0+$/, "") : String(Math.round(next)));
  };
  return <div className="paper-number-stepper">
    <input aria-label={ariaLabel} type="number" min={min} max={max} step={step} value={value} onChange={(event) => onChange(event.currentTarget.value)} />
    <span className="paper-stepper-buttons">
      <button type="button" aria-label={`${ariaLabel} 늘리기`} onClick={() => adjust(1)}>＋</button>
      <button type="button" aria-label={`${ariaLabel} 줄이기`} onClick={() => adjust(-1)}>−</button>
    </span>
  </div>;
}

function OrderTicket({ market, symbol, currency, onAccountChanged, onOrdersChanged, compact = false }: { market: Market; symbol: string; currency: string; onAccountChanged: (snapshot: PaperAccountSnapshot) => void; onOrdersChanged?: () => void | Promise<void>; compact?: boolean }) {
  const [side, setSide] = useState<Side>("buy");
  const [orderType, setOrderType] = useState<"market" | "limit">("market");
  const [quantity, setQuantity] = useState("1");
  const [limitPrice, setLimitPrice] = useState("");
  const [quote, setQuote] = useState<MarketQuote | null>(null);
  const [costs, setCosts] = useState<Costs>({ ...MARKET_COST_PRESETS[market].costs });
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const resolvedSymbol = resolveSymbol(symbol);

  useEffect(() => { setQuote(null); setMessage(null); }, [market, resolvedSymbol]);
  useEffect(() => { setCosts({ ...MARKET_COST_PRESETS[market].costs }); }, [market]);

  const refreshQuote = async () => {
    setBusy(true); setMessage(null);
    try { setQuote(await invoke<MarketQuote>(market === "coin" ? "upbit_market_quote" : "toss_market_quote", { symbol: resolvedSymbol })); }
    catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const displayAmount = Number(quantity);
    const amount = market === "coin" ? Math.round(displayAmount * 100_000_000) : displayAmount;
    if (!Number.isFinite(displayAmount) || displayAmount <= 0 || !Number.isSafeInteger(amount) || amount <= 0 || busy) return;
    setBusy(true); setMessage(null);
    try {
      if (orderType === "market") {
        const command = market === "coin" ? "upbit_execute_paper_market_order" : "toss_execute_paper_market_order";
        const request = market === "coin"
          ? { symbol: resolvedSymbol, side, quantity: amount, idempotencyKey: createOrderId("coin-market"), costs }
          : { symbol: resolvedSymbol, expectedCurrency: currency, side, quantity: amount, idempotencyKey: createOrderId("market"), costs };
        const account = await invoke<PaperAccountSnapshot>(command, {
          request,
        });
        onAccountChanged(account);
        setMessage(`${resolvedSymbol} ${side === "buy" ? "매수" : "매도"} ${displayAmount}${market === "coin" ? "개" : "주"}가 내부 원장에 체결되었습니다.`);
        await refreshQuote();
      } else {
        const limitPriceMinor = toMinor(limitPrice, currency);
        if (!limitPriceMinor) throw new Error("유효한 지정가를 입력해 주세요.");
        await invoke<ManualOrder>("manual_paper_limit_order_submit", { request: {
          orderId: createOrderId("limit"), market, symbol: resolvedSymbol, currency, side, quantity: amount, limitPriceMinor,
        } });
        await onOrdersChanged?.();
        setMessage("지정가 주문을 로컬 대기 상태로 저장했습니다. KIS 모의 어댑터 연결 전에는 외부로 전송되지 않습니다.");
      }
    } catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  };

  return <form className={`paper-order-ticket ${compact ? "is-compact" : ""}`} onSubmit={submit}>
    <div className="paper-side-tabs" role="group" aria-label="주문 방향">
      <button type="button" className={side === "buy" ? "is-buy" : ""} onClick={() => setSide("buy")}>매수</button>
      <button type="button" className={side === "sell" ? "is-sell" : ""} onClick={() => setSide("sell")}>매도</button>
    </div>
    <div className="paper-quote-line"><span>{resolvedSymbol || "종목 미선택"}</span><strong>{quote ? formatMoney(quote.lastPriceMinor, quote.currency) : "현재가 미조회"}</strong><button type="button" onClick={() => void refreshQuote()} disabled={busy || !resolvedSymbol}>{busy ? "조회 중" : "현재가"}</button></div>
    <label>주문 방식<select value={orderType} onChange={(event) => setOrderType(event.currentTarget.value as "market" | "limit")}><option value="market">시장가 · 즉시 내부체결</option><option value="limit">지정가 · 대기/취소</option></select></label>
    <label>{market === "coin" ? "수량 (최소 0.00000001개)" : "수량"}<NumericStepper ariaLabel="주문 수량" min={market === "coin" ? 0.00000001 : 1} step={market === "coin" ? 0.00000001 : 1} value={quantity} onChange={setQuantity} /></label>
    {orderType === "limit" && <label>지정가<input inputMode="decimal" value={limitPrice} onChange={(event) => setLimitPrice(event.currentTarget.value)} placeholder={currency === "KRW" ? "70000" : "210.50"} /></label>}
    {!compact && <fieldset className="paper-cost-fields"><legend>공식 기본 비용 (bp) · 직접 변경 가능</legend>{Object.entries({ buyFeeBps: "매수 수수료", sellFeeBps: "매도 수수료", sellTaxBps: "매도 세금·제비용", slippageBps: "슬리피지 가정" }).map(([key, label]) => <label key={key}>{label}<NumericStepper ariaLabel={label} min={0} max={9999.999} step={0.001} value={String(costs[key as keyof Costs])} onChange={(value) => setCosts((current) => ({ ...current, [key]: Math.max(0, Number(value) || 0) }))} /></label>)}<button className="paper-cost-reset" type="button" onClick={() => setCosts({ ...MARKET_COST_PRESETS[market].costs })}>공식 기본값 복원</button></fieldset>}
    {!compact && <div className="paper-cost-note"><strong>{MARKET_COST_PRESETS[market].source}</strong><span>{MARKET_COST_PRESETS[market].basis}</span><span>슬리피지는 고정 부과율이 아니므로 기본 0bp이며 전략·유동성에 맞게 직접 변경합니다.</span></div>}
    {message && <p className="paper-order-message" role="status">{message}</p>}
    <button className={side === "buy" ? "paper-submit-buy" : "paper-submit-sell"} type="submit" disabled={busy || !resolvedSymbol}>{busy ? "처리 중…" : `${side === "buy" ? "매수" : "매도"} 모의주문`}</button>
  </form>;
}

export function KisPaperConnection() {
  const [status, setStatus] = useState<KisStatus | null>(null);
  const [form, setForm] = useState({ appKey: "", appSecret: "", htsId: "", accountNumber: "", productCode: "01" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [account, setAccount] = useState<{ deposit?: string | null; totalEvaluationAmount?: string | null; positions: Array<{ symbol: string; name: string; quantity: string }> } | null>(null);
  const [remoteOrder, setRemoteOrder] = useState({ symbol: "005930", side: "buy" as Side, quantity: "1", orderType: "limit" as "market" | "limit", price: "", confirmation: "" });
  const openOfficialPage = async (target: "kis-paper-account" | "kis-api-portal") => {
    setError(null);
    try { await invoke("open_official_external_page", { target }); }
    catch (reason) { setError(String(reason)); }
  };
  useEffect(() => { if (isTauriRuntime) void invoke<KisStatus>("kis_paper_config_status").then(setStatus).catch((reason) => setError(String(reason))); }, []);
  const save = async (event: FormEvent) => {
    event.preventDefault(); setBusy(true); setError(null);
    try { setStatus(await invoke<KisStatus>("kis_paper_config_save", { request: form })); setForm({ appKey: "", appSecret: "", htsId: "", accountNumber: "", productCode: "01" }); }
    catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };
  const remove = async () => { setBusy(true); setError(null); try { setStatus(await invoke<KisStatus>("kis_paper_config_delete")); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } };
  const verify = async () => { setBusy(true); setError(null); try { setAccount(await invoke("kis_paper_account_snapshot")); } catch (reason) { setAccount(null); setError(String(reason)); } finally { setBusy(false); } };
  const submitRemoteOrder = async (event: FormEvent) => { event.preventDefault(); setBusy(true); setError(null); try { const result = await invoke<{ status: string; remoteOrderId?: string | null; message: string }>("kis_paper_order_submit", { request: { requestId: createOrderId("kis-paper"), symbol: remoteOrder.symbol, side: remoteOrder.side, quantity: Number(remoteOrder.quantity), orderType: remoteOrder.orderType, price: remoteOrder.orderType === "market" ? 0 : Number(remoteOrder.price), confirmation: remoteOrder.confirmation } }); setError(`${result.status} · 주문번호 ${result.remoteOrderId ?? "미제공"} · ${result.message}`); setRemoteOrder((current) => ({ ...current, confirmation: "" })); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } };
  return <section className="paper-kis-connect">
    <header><div><span>KIS VIRTUAL TRADING</span><h3>KIS 모의계좌 연결 준비</h3></div><strong className={status?.configured ? "is-ready" : ""}>{status?.configured ? "저장됨" : "미연결"}</strong></header>
    {!status?.configured && <section className="paper-kis-issuance" aria-labelledby="kis-issuance-title">
      <div><span>STEP 1 · OFFICIAL KIS</span><h4 id="kis-issuance-title">모의계좌 발급</h4></div>
      <ol>
        <li>한국투자증권 모의투자 서비스에 신청합니다.</li>
        <li>KIS Developers에서 모의투자용 App Key·Secret을 준비합니다.</li>
        <li>발급된 모의계좌 앞 8자리와 상품코드 01을 아래에 입력합니다.</li>
      </ol>
      <div className="paper-kis-issuance-actions">
        <button type="button" onClick={() => void openOfficialPage("kis-paper-account")}>모의계좌 발급 페이지</button>
        <button type="button" onClick={() => void openOfficialPage("kis-api-portal")}>Open API 신청 페이지</button>
      </div>
      <small>공식 페이지는 기본 브라우저에서 열립니다. 키와 계좌번호는 채팅에 공유하지 마세요.</small>
    </section>}
    <p>{status?.message ?? "연결 상태를 확인하고 있습니다."}</p>
    {status?.configured ? <><div className="paper-kis-saved"><span>{status.maskedAccountNumber}-{status.productCode}</span><button type="button" onClick={() => void verify()} disabled={busy}>{busy ? "조회 중…" : "모의 잔고 연결 확인"}</button><button type="button" onClick={() => void remove()} disabled={busy}>저장 정보 삭제</button></div>{account && <div className="paper-kis-account" role="status"><strong>예수금 {account.deposit ?? "미제공"} · 평가액 {account.totalEvaluationAmount ?? "미제공"}</strong><span>보유 {account.positions.length}종목</span></div>}<form className="paper-kis-order" onSubmit={submitRemoteOrder}><h4>KIS 서버 모의주문 · 국내주식</h4><label>종목코드<input inputMode="numeric" maxLength={6} value={remoteOrder.symbol} onChange={(event) => setRemoteOrder((current) => ({ ...current, symbol: event.currentTarget.value.replace(/\D/g, "") }))} /></label><label>방향<select value={remoteOrder.side} onChange={(event) => setRemoteOrder((current) => ({ ...current, side: event.currentTarget.value as Side }))}><option value="buy">매수</option><option value="sell">매도</option></select></label><label>주문방식<select value={remoteOrder.orderType} onChange={(event) => setRemoteOrder((current) => ({ ...current, orderType: event.currentTarget.value as "market" | "limit" }))}><option value="limit">지정가</option><option value="market">시장가</option></select></label><label>수량<input type="number" min="1" step="1" value={remoteOrder.quantity} onChange={(event) => setRemoteOrder((current) => ({ ...current, quantity: event.currentTarget.value }))} /></label>{remoteOrder.orderType === "limit" && <label>가격<input type="number" min="1" step="1" value={remoteOrder.price} onChange={(event) => setRemoteOrder((current) => ({ ...current, price: event.currentTarget.value }))} /></label>}<label>확인 문구<input value={remoteOrder.confirmation} onChange={(event) => setRemoteOrder((current) => ({ ...current, confirmation: event.currentTarget.value }))} placeholder="KIS 모의주문 전송" /></label><button type="submit" disabled={busy || remoteOrder.confirmation !== "KIS 모의주문 전송"}>{busy ? "전송 중…" : "KIS 모의주문 전송"}</button></form></> : <form onSubmit={save}>
      <label>모의 App Key<input value={form.appKey} onChange={(event) => setForm((current) => ({ ...current, appKey: event.currentTarget.value }))} autoComplete="off" /></label>
      <label>모의 App Secret<input type="password" value={form.appSecret} onChange={(event) => setForm((current) => ({ ...current, appSecret: event.currentTarget.value }))} autoComplete="new-password" /></label>
      <label>HTS ID<input value={form.htsId} onChange={(event) => setForm((current) => ({ ...current, htsId: event.currentTarget.value }))} /></label>
      <label>모의계좌 앞 8자리<input inputMode="numeric" maxLength={8} value={form.accountNumber} onChange={(event) => setForm((current) => ({ ...current, accountNumber: event.currentTarget.value.replace(/\D/g, "") }))} /></label>
      <label>상품코드<input inputMode="numeric" maxLength={2} value={form.productCode} onChange={(event) => setForm((current) => ({ ...current, productCode: event.currentTarget.value.replace(/\D/g, "") }))} /></label>
      <button type="submit" disabled={busy || Object.values(form).some((value) => !value)}>{busy ? "저장 중…" : "이 PC에 안전하게 저장"}</button>
    </form>}
    <small>계좌번호만으로는 연결할 수 없습니다. 모의 App Key·Secret도 필요하며 Windows 자격 증명 관리자에만 저장됩니다. 잔고 조회는 KIS 모의 서버에만 연결되고 실계좌 주문은 잠겨 있습니다.</small>
    {error && <p role="alert">{error}</p>}
  </section>;
}

export function PaperTradingTerminal({ onAccountChanged }: { onAccountChanged: (snapshot: PaperAccountSnapshot) => void }) {
  const [terminalMode, setTerminalMode] = useState<Market | "futures">("kr");
  const [market, setMarket] = useState<Market>("kr");
  const [symbolInput, setSymbolInput] = useState(MARKET_INFO.kr.defaultSymbol);
  const [confirmedSymbol, setConfirmedSymbol] = useState(MARKET_INFO.kr.defaultSymbol);
  const [interval, setInterval] = useState<Interval>("1d");
  const [count, setCount] = useState("120");
  const [adjusted, setAdjusted] = useState(true);
  const [indicators, setIndicators] = useState(new Set(["ma5", "ma20", "volume"]));
  const [indicatorPanelOpen, setIndicatorPanelOpen] = useState(true);
  const [chart, setChart] = useState<ChartSnapshot | null>(null);
  const [accounts, setAccounts] = useState<PaperAccountSnapshot[]>([]);
  const [orders, setOrders] = useState<ManualOrder[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const selectedCurrency = chart?.currency ?? MARKET_INFO[market].currency;
  const selectedAccount = accounts.find((item) => item.account.currency === selectedCurrency);

  const refreshAccountsAndOrders = async () => {
    const [accountSnapshot, manualOrders] = await Promise.all([
      invoke<AccountsSnapshot>("paper_accounts_status"), invoke<ManualOrder[]>("manual_paper_orders"),
    ]);
    setAccounts(accountSnapshot.accounts); setOrders(manualOrders);
  };
  useEffect(() => { if (isTauriRuntime) void refreshAccountsAndOrders().catch((reason) => setError(String(reason))); }, []);

  const changeMarket = (next: Market) => {
    setTerminalMode(next);
    setMarket(next); setSymbolInput(MARKET_INFO[next].defaultSymbol); setConfirmedSymbol(MARKET_INFO[next].defaultSymbol); setChart(null); setError(null); if (next === "coin") setAdjusted(false);
  };
  const loadChart = async (event?: FormEvent) => {
    event?.preventDefault();
    const symbol = resolveSymbol(symbolInput);
    setConfirmedSymbol(symbol);
    setLoading(true); setError(null);
    try {
      const command = market === "coin" ? "upbit_chart_snapshot" : "toss_chart_snapshot";
      const requestedCount = market === "coin" ? Math.min(Number(count), 200) : Number(count);
      const snapshot = await invoke<ChartSnapshot>(command, { request: { symbol, interval, count: requestedCount, adjusted: market === "coin" ? false : adjusted } });
      if ((market === "kr" && snapshot.currency !== "KRW") || (market === "us" && snapshot.currency !== "USD")) throw new Error("선택한 시장과 조회된 종목의 통화가 일치하지 않습니다.");
      setChart(snapshot);
    } catch (reason) { setChart(null); setError(String(reason)); }
    finally { setLoading(false); }
  };
  const toggleIndicator = (indicator: string) => setIndicators((current) => { const next = new Set(current); if (next.has(indicator)) next.delete(indicator); else next.add(indicator); return next; });
  const cancelOrder = async (orderId: string) => { try { await invoke("manual_paper_order_cancel", { orderId }); await refreshAccountsAndOrders(); } catch (reason) { setError(String(reason)); } };
  const handleAccountChanged = (snapshot: PaperAccountSnapshot) => { onAccountChanged(snapshot); setAccounts((current) => [...current.filter((item) => item.account.currency !== snapshot.account.currency), snapshot]); void refreshAccountsAndOrders(); };

  if (terminalMode === "futures") return <main className="paper-terminal" aria-label="모의투자 터미널"><header className="paper-terminal-header"><div><p className="eyebrow">PAPER TRADING TERMINAL</p><h2>통합 모의투자</h2></div><div className="paper-market-tabs" role="tablist" aria-label="시장 선택">{(Object.keys(MARKET_INFO) as Market[]).map((key) => <button type="button" role="tab" aria-selected={false} key={key} onClick={() => changeMarket(key)}>{MARKET_INFO[key].label}</button>)}<button type="button" role="tab" aria-selected={true} className="is-active">선물</button></div><span className="paper-live-lock">INTERNAL ONLY · 증권사 미연결</span></header><FuturesPaperPanel /></main>;

  return <main className="paper-terminal" aria-label="모의투자 터미널">
    <header className="paper-terminal-header"><div><p className="eyebrow">PAPER TRADING TERMINAL</p><h2>통합 모의투자</h2></div><div className="paper-market-tabs" role="tablist" aria-label="시장 선택">{(Object.keys(MARKET_INFO) as Market[]).map((key) => <button type="button" role="tab" aria-selected={terminalMode === key} className={terminalMode === key ? "is-active" : ""} key={key} onClick={() => changeMarket(key)}>{MARKET_INFO[key].label}</button>)}<button type="button" role="tab" aria-selected="false" onClick={() => setTerminalMode("futures")}>선물</button></div><span className="paper-live-lock">SHADOW ONLY · 실주문 잠금</span></header>
    <section className="paper-terminal-grid">
      <div className="paper-chart-workspace">
        <form className="paper-symbol-bar" onSubmit={loadChart}>
          <div className="paper-symbol-field"><label htmlFor="paper-terminal-symbol">종목명·티커·코드</label><SymbolSearchInput id="paper-terminal-symbol" inputRef={searchRef} market={market} value={symbolInput} onChange={setSymbolInput} onSelect={(item) => { setSymbolInput(item.symbol); setConfirmedSymbol(item.symbol); setChart(null); setError(null); }} placeholder={MARKET_INFO[market].hint} /></div>
          <select aria-label="캔들 주기" value={interval} onChange={(event) => setInterval(event.currentTarget.value as Interval)}><option value="1d">일봉</option><option value="1m">1분봉</option></select>
          <select aria-label="캔들 개수" value={count} onChange={(event) => setCount(event.currentTarget.value)}><option value="60">60봉</option><option value="120">120봉</option><option value="200">200봉</option><option value="500">500봉</option></select>
          <label className="paper-adjusted"><input type="checkbox" checked={adjusted} disabled={market === "coin"} onChange={(event) => setAdjusted(event.currentTarget.checked)} />수정주가</label>
          <button type="submit" disabled={loading}>{loading ? "불러오는 중…" : "차트 불러오기"}</button>
        </form>
        <div className="paper-indicator-bar"><strong>보조지표</strong>{INDICATOR_DEFINITIONS.filter((item) => ["ma5", "ma20", "ma60", "volume", "rsi", "macd"].includes(item.id)).map((item) => <label key={item.id}><input type="checkbox" checked={indicators.has(item.id)} onChange={() => toggleIndicator(item.id)} />{item.label}</label>)}<button type="button" aria-expanded={indicatorPanelOpen} onClick={() => setIndicatorPanelOpen((current) => !current)}>{indicatorPanelOpen ? "지표 설정 닫기" : "전체 지표 설정"}</button></div>
        <div className="paper-chart-frame">{loading ? <div className="paper-chart-state" role="status">{market === "coin" ? "Upbit" : "토스증권"} 캔들을 불러오고 있습니다.</div> : chart ? <InteractiveCandleChart snapshot={chart} indicators={indicators} /> : <div className="paper-chart-state"><strong>차트 대기</strong><p>종목을 확정하고 차트를 불러오세요.</p></div>}</div>
        {error && <div className="paper-terminal-error" role="alert"><strong>확인 필요</strong><span>{error}</span><button type="button" onClick={() => { setError(null); searchRef.current?.focus(); }}>입력 수정</button></div>}
        <section className="paper-orders"><header><div><span>OPEN / CANCELLED</span><h3>지정가 모의주문</h3></div><button type="button" onClick={() => void refreshAccountsAndOrders()}>새로고침</button></header>{orders.length ? <table><thead><tr><th>시장</th><th>종목</th><th>방향</th><th>수량</th><th>지정가</th><th>상태</th><th>작업</th></tr></thead><tbody>{orders.map((order) => <tr key={order.orderId}><td>{MARKET_INFO[order.market].label}</td><td>{order.symbol}</td><td>{order.side === "buy" ? "매수" : "매도"}</td><td>{order.quantity / order.quantityScale}</td><td>{formatMoney(order.limitPriceMinor, order.currency)}</td><td>{order.status === "pending" ? "대기" : "취소"}</td><td>{order.status === "pending" ? <button type="button" onClick={() => void cancelOrder(order.orderId)}>주문 취소</button> : <span>완료</span>}</td></tr>)}</tbody></table> : <p>대기 중인 지정가 주문이 없습니다. 시장가 주문은 즉시 내부 체결되어 취소 대상이 아닙니다.</p>}</section>
        {indicatorPanelOpen && <section className="paper-indicator-catalog" aria-labelledby="indicator-catalog-title"><header><div><span>CHART STUDIES</span><h3 id="indicator-catalog-title">보조지표 설정</h3></div><button type="button" onClick={() => setIndicators(new Set())}>전체 해제</button></header><p>현재 차트의 OHLCV로 로컬 계산합니다. 선택은 앱을 닫기 전까지 유지되고, 직접 그린 추세선은 종목·봉 주기별로 자동 저장됩니다.</p><div>{(["price", "oscillator", "volume", "telegram"] as const).map((group) => <fieldset key={group}><legend>{({ price: "가격 차트", oscillator: "오실레이터", volume: "거래량·수급", telegram: "텔레그램 수식" } as const)[group]}</legend>{INDICATOR_DEFINITIONS.filter((item) => item.group === group).map((item) => <label key={item.id} title={item.dataNote ?? item.description}><input type="checkbox" checked={indicators.has(item.id)} onChange={() => toggleIndicator(item.id)} /><span><strong>{item.label}</strong><small>{item.description}{item.dataNote ? ` · ${item.dataNote}` : ""}</small></span></label>)}</fieldset>)}</div><aside><strong>데이터 한계</strong><span>토스·Upbit의 OHLCV를 사용한 표시용 계산입니다. 텔레그램 원본 Pine 코드는 실행하지 않으며, 기관 순매수·호가·체결 방향 원자료가 필요한 값은 만들어내지 않습니다.</span></aside></section>}
      </div>
      <aside className="paper-trade-sidebar">
        <section className="paper-account-summary"><header><span>INTERNAL PAPER</span><strong>{selectedCurrency}</strong></header><p>{selectedAccount ? formatMoney(selectedAccount.account.cashMinor, selectedCurrency) : "계좌 확인 중"}</p><dl><div><dt>포지션</dt><dd>{selectedAccount ? Object.keys(selectedAccount.account.positions).length : 0}개</dd></div><div><dt>실현손익</dt><dd>{selectedAccount ? formatMoney(selectedAccount.account.realizedPnlMinor, selectedCurrency) : "-"}</dd></div><div><dt>원장 사건</dt><dd>{selectedAccount?.account.eventCount ?? 0}건</dd></div></dl></section>
        <OrderTicket market={market} symbol={confirmedSymbol} currency={selectedCurrency} onAccountChanged={handleAccountChanged} onOrdersChanged={refreshAccountsAndOrders} />
        <section className="paper-provider-status"><strong>{market === "coin" ? "UPBIT PUBLIC DATA" : "TOSS MARKET DATA"}</strong><p>{market === "coin" ? "Upbit 공개 시세 → 내부 SQLite 모의원장 · 실제 거래소 주문 없음" : "토스증권 읽기 전용 시세 → 내부 SQLite 모의원장"}</p></section>
      </aside>
    </section>
  </main>;
}

export function QuickPaperOrder({ onAccountChanged }: { onAccountChanged: (snapshot: PaperAccountSnapshot) => void }) {
  const [market, setMarket] = useState<Market>("kr");
  const [symbol, setSymbol] = useState(MARKET_INFO.kr.defaultSymbol);
  const confirmed = useMemo(() => resolveSymbol(symbol), [symbol]);
  const changeMarket = (next: Market) => { setMarket(next); setSymbol(MARKET_INFO[next].defaultSymbol); };
  return <section className="quick-paper-order" aria-labelledby="quick-paper-title"><header><div><span>QUICK PAPER ORDER</span><h3 id="quick-paper-title">간편 모의주문</h3></div><strong>실전 잠금</strong></header><div className="quick-market-tabs">{(Object.keys(MARKET_INFO) as Market[]).map((key) => <button type="button" className={market === key ? "is-active" : ""} onClick={() => changeMarket(key)} key={key}>{MARKET_INFO[key].label}</button>)}</div><div className="quick-symbol-field"><label htmlFor="quick-paper-symbol">종목명·티커·코드</label><SymbolSearchInput id="quick-paper-symbol" market={market} value={symbol} onChange={setSymbol} onSelect={(item) => setSymbol(item.symbol)} placeholder={MARKET_INFO[market].hint} /></div><OrderTicket compact market={market} symbol={confirmed} currency={MARKET_INFO[market].currency} onAccountChanged={onAccountChanged} /><p>상세 차트·지표·지정가 취소·계좌 연결은 왼쪽 모의 메뉴에서 관리합니다.</p></section>;
}
