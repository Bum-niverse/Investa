import { invoke } from "@tauri-apps/api/core";
import { type FormEvent, useEffect, useState } from "react";

type FuturesPosition = {
  positionId: string;
  symbol: string;
  name: string;
  kind: "stock" | "index";
  side: "long" | "short";
  contracts: number;
  entryPriceMinor: number;
  markPriceMinor: number;
  contractMultiplier: number;
  priceScale: number;
  initialMarginBps: number;
  maintenanceMarginBps: number;
  reservedMarginMinor: number;
  unrealizedPnlMinor: number;
  maintenanceRequiredMinor: number;
  liquidationWarning: boolean;
};

type FuturesSnapshot = {
  currency: "KRW";
  initialCashMinor: number;
  availableCashMinor: number;
  equityMinor: number;
  reservedMarginMinor: number;
  unrealizedPnlMinor: number;
  realizedPnlMinor: number;
  positions: FuturesPosition[];
  eventCount: number;
  liveOrderEnabled: false;
  warning: string;
};

const isTauriRuntime = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const money = (value: number) => new Intl.NumberFormat("ko-KR", { style: "currency", currency: "KRW", maximumFractionDigits: 0 }).format(value);
const displayPrice = (minor: number, scale: number) => (minor / scale).toLocaleString("ko-KR", { maximumFractionDigits: 4 });
const requestId = (prefix: string) => `${prefix}-${Date.now()}-${crypto.randomUUID()}`;

export function FuturesPaperPanel() {
  const [snapshot, setSnapshot] = useState<FuturesSnapshot | null>(null);
  const [form, setForm] = useState({ symbol: "KOSPI200-DEMO", name: "지수선물 연습상품", kind: "index" as "stock" | "index", side: "long" as "long" | "short", contracts: "1", price: "350.00", multiplier: "250000", initialMarginPercent: "15", maintenanceMarginPercent: "10", fee: "0" });
  const [prices, setPrices] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const load = async () => {
    if (!isTauriRuntime) { setMessage("데스크톱 앱에서 내부 선물 모의원장을 사용할 수 있습니다."); return; }
    try { setSnapshot(await invoke<FuturesSnapshot>("futures_paper_status")); }
    catch (reason) { setMessage(String(reason)); }
  };
  useEffect(() => { void load(); }, []);

  const scaledPrice = (value: string) => {
    const numeric = Number(value);
    return Number.isFinite(numeric) && numeric > 0 ? Math.round(numeric * 100) : null;
  };
  const open = async (event: FormEvent) => {
    event.preventDefault();
    const entryPriceMinor = scaledPrice(form.price);
    if (!entryPriceMinor) { setMessage("유효한 진입가격을 입력해 주세요."); return; }
    setBusy(true); setMessage(null);
    try {
      const next = await invoke<FuturesSnapshot>("futures_paper_open", { request: {
        requestId: requestId("futures-open"), symbol: form.symbol, name: form.name, kind: form.kind, side: form.side,
        contracts: Number(form.contracts), entryPriceMinor, priceScale: 100, contractMultiplier: Number(form.multiplier),
        initialMarginBps: Math.round(Number(form.initialMarginPercent) * 100), maintenanceMarginBps: Math.round(Number(form.maintenanceMarginPercent) * 100), feeMinor: Number(form.fee),
      } });
      setSnapshot(next); setMessage("내부 선물 모의포지션을 개설했습니다. 외부 주문은 전송되지 않았습니다.");
    } catch (reason) { setMessage(String(reason)); } finally { setBusy(false); }
  };
  const mark = async (position: FuturesPosition) => {
    const markPriceMinor = scaledPrice(prices[position.positionId] ?? displayPrice(position.markPriceMinor, position.priceScale));
    if (!markPriceMinor) { setMessage("유효한 평가가격을 입력해 주세요."); return; }
    setBusy(true); setMessage(null);
    try { setSnapshot(await invoke<FuturesSnapshot>("futures_paper_mark", { request: { requestId: requestId("futures-mark"), positionId: position.positionId, markPriceMinor } })); }
    catch (reason) { setMessage(String(reason)); } finally { setBusy(false); }
  };
  const close = async (position: FuturesPosition) => {
    const exitPriceMinor = scaledPrice(prices[position.positionId] ?? displayPrice(position.markPriceMinor, position.priceScale));
    if (!exitPriceMinor) { setMessage("유효한 청산가격을 입력해 주세요."); return; }
    setBusy(true); setMessage(null);
    try { setSnapshot(await invoke<FuturesSnapshot>("futures_paper_close", { request: { requestId: requestId("futures-close"), positionId: position.positionId, exitPriceMinor, feeMinor: Number(form.fee) || 0 } })); setMessage("포지션을 내부 모의원장에서 청산했습니다."); }
    catch (reason) { setMessage(String(reason)); } finally { setBusy(false); }
  };

  return <section className="futures-paper" aria-labelledby="futures-paper-title">
    <header><div><p className="eyebrow">DERIVATIVES PAPER SANDBOX</p><h2 id="futures-paper-title">주식선물·지수선물 내부 모의투자</h2></div><span>INTERNAL ONLY · 증권사 미연결</span></header>
    <div className="futures-summary" aria-label="선물 모의계좌 요약">
      <div><small>계좌자산</small><strong>{snapshot ? money(snapshot.equityMinor) : "확인 중"}</strong></div>
      <div><small>가용현금</small><strong>{snapshot ? money(snapshot.availableCashMinor) : "-"}</strong></div>
      <div><small>예약증거금</small><strong>{snapshot ? money(snapshot.reservedMarginMinor) : "-"}</strong></div>
      <div><small>미실현손익</small><strong>{snapshot ? money(snapshot.unrealizedPnlMinor) : "-"}</strong></div>
    </div>
    <div className="futures-layout">
      <form className="futures-ticket" onSubmit={open}>
        <h3>연습 계약 입력</h3>
        <p>실제 상품 규격을 자동 적용하지 않습니다. 거래소에서 확인한 가격·승수·증거금률을 직접 입력해야 합니다.</p>
        <label>상품 구분<select value={form.kind} onChange={(event) => setForm((current) => ({ ...current, kind: event.currentTarget.value as "stock" | "index" }))}><option value="index">지수선물</option><option value="stock">주식선물</option></select></label>
        <label>종목코드<input value={form.symbol} maxLength={24} onChange={(event) => setForm((current) => ({ ...current, symbol: event.currentTarget.value.toUpperCase().replace(/[^A-Z0-9_.-]/g, "") }))} /></label>
        <label>표시 이름<input value={form.name} maxLength={60} onChange={(event) => setForm((current) => ({ ...current, name: event.currentTarget.value }))} /></label>
        <label>방향<select value={form.side} onChange={(event) => setForm((current) => ({ ...current, side: event.currentTarget.value as "long" | "short" }))}><option value="long">롱 · 상승</option><option value="short">숏 · 하락</option></select></label>
        <label>계약 수<input type="number" min="1" max="100" step="1" value={form.contracts} onChange={(event) => setForm((current) => ({ ...current, contracts: event.currentTarget.value }))} /></label>
        <label>진입가격<input type="number" min="0.01" step="0.01" value={form.price} onChange={(event) => setForm((current) => ({ ...current, price: event.currentTarget.value }))} /></label>
        <label>계약승수 (원/1가격단위)<input type="number" min="1" step="1" value={form.multiplier} onChange={(event) => setForm((current) => ({ ...current, multiplier: event.currentTarget.value }))} /></label>
        <label>개시증거금률 %<input type="number" min="0.01" max="100" step="0.01" value={form.initialMarginPercent} onChange={(event) => setForm((current) => ({ ...current, initialMarginPercent: event.currentTarget.value }))} /></label>
        <label>유지증거금률 %<input type="number" min="0.01" max="100" step="0.01" value={form.maintenanceMarginPercent} onChange={(event) => setForm((current) => ({ ...current, maintenanceMarginPercent: event.currentTarget.value }))} /></label>
        <label>편도 수수료 가정 (원)<input type="number" min="0" step="1" value={form.fee} onChange={(event) => setForm((current) => ({ ...current, fee: event.currentTarget.value }))} /></label>
        <button type="submit" disabled={busy}>{busy ? "처리 중…" : "내부 모의포지션 개설"}</button>
      </form>
      <section className="futures-positions"><header><div><span>APPEND-ONLY LEDGER</span><h3>보유 포지션</h3></div><button type="button" onClick={() => void load()} disabled={busy}>새로고침</button></header>
        {snapshot?.positions.length ? <div className="futures-position-list">{snapshot.positions.map((position) => <article className={position.liquidationWarning ? "is-warning" : ""} key={position.positionId}>
          <header><div><small>{position.kind === "index" ? "지수선물" : "주식선물"} · {position.symbol}</small><strong>{position.name}</strong></div><b>{position.side === "long" ? "LONG" : "SHORT"} {position.contracts}계약</b></header>
          <dl><div><dt>진입/평가</dt><dd>{displayPrice(position.entryPriceMinor, position.priceScale)} / {displayPrice(position.markPriceMinor, position.priceScale)}</dd></div><div><dt>미실현손익</dt><dd>{money(position.unrealizedPnlMinor)}</dd></div><div><dt>예약/유지증거금</dt><dd>{money(position.reservedMarginMinor)} / {money(position.maintenanceRequiredMinor)}</dd></div></dl>
          {position.liquidationWarning && <p role="alert">유지증거금 미달 경고 · 실제 강제청산 주문은 생성하지 않습니다.</p>}
          <div className="futures-position-actions"><label>평가·청산가격<input type="number" min="0.01" step="0.01" value={prices[position.positionId] ?? displayPrice(position.markPriceMinor, position.priceScale).replace(/,/g, "")} onChange={(event) => setPrices((current) => ({ ...current, [position.positionId]: event.currentTarget.value }))} /></label><button type="button" onClick={() => void mark(position)} disabled={busy}>시가평가</button><button type="button" onClick={() => void close(position)} disabled={busy}>모의청산</button></div>
        </article>)}</div> : <div className="futures-empty"><strong>열린 선물 포지션 없음</strong><p>왼쪽에서 연습 계약을 입력하면 증거금과 손익을 내부 원장으로 계산합니다.</p></div>}
      </section>
    </div>
    {message && <p className="futures-message" role="status">{message}</p>}
    <footer>{snapshot?.warning ?? "초기 계좌를 준비하고 있습니다."} · 사건 {snapshot?.eventCount ?? 0}건</footer>
  </section>;
}
