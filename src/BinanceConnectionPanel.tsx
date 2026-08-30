import { type FormEvent, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export type BinanceConnectionStatus = {
  configured: boolean;
  connected: boolean;
  message: string;
};

export type BinanceAccountSection = {
  connected: boolean;
  message: string;
  balances: Array<{ asset: string; walletBalance: string; availableBalance: string; unrealizedProfit: string }>;
  positions: Array<{ symbol: string; positionAmount: string; entryPrice: string; markPrice: string; unrealizedProfit: string; liquidationPrice: string; leverage: string; marginType: string }>;
};

export type BinanceAccountSnapshot = {
  provider: string;
  fetchedAtMs: number;
  readOnly: boolean;
  permissionVerified: boolean;
  permissionMessage: string;
  spot: BinanceAccountSection;
  usdM: BinanceAccountSection;
  coinM: BinanceAccountSection;
};

type BinancePublicSnapshot = {
  fetchedAtMs: number;
  spot: { market: string; symbol: string; price: string };
  usdM: { market: string; symbol: string; price: string };
  coinM: { market: string; symbol: string; price: string };
};

type Props = {
  status: BinanceConnectionStatus;
  refreshedSnapshot?: BinanceAccountSnapshot | null;
  onStatusChange: (status: BinanceConnectionStatus, snapshot?: BinanceAccountSnapshot | null) => void;
};

const isTauriRuntime = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const formatValue = (value: string) => Number(value).toLocaleString("ko-KR", { maximumFractionDigits: 8 });

export function BinanceConnectionPanel({ status, refreshedSnapshot, onStatusChange }: Props) {
  const [apiKey, setApiKey] = useState("");
  const [secretKey, setSecretKey] = useState("");
  const [publicSnapshot, setPublicSnapshot] = useState<BinancePublicSnapshot | null>(null);
  const [publicError, setPublicError] = useState<string | null>(null);
  const [accountSnapshot, setAccountSnapshot] = useState<BinanceAccountSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauriRuntime) return;
    let disposed = false;
    void invoke<BinancePublicSnapshot>("binance_public_snapshot")
      .then((snapshot) => { if (!disposed) { setPublicSnapshot(snapshot); setPublicError(null); } })
      .catch(() => { if (!disposed) { setPublicSnapshot(null); setPublicError("Binance 공개 시세를 확인하지 못했습니다."); } });
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    if (refreshedSnapshot !== undefined) setAccountSnapshot(refreshedSnapshot);
  }, [refreshedSnapshot]);

  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (!apiKey || !secretKey || busy) return;
    setBusy(true);
    setError(null);
    try {
      const snapshot = await invoke<BinanceAccountSnapshot>("binance_save_credentials", { request: { apiKey, secretKey } });
      setAccountSnapshot(snapshot);
      setApiKey("");
      setSecretKey("");
      onStatusChange({ configured: true, connected: true, message: "상품별 읽기 전용 계좌 연결을 확인했습니다." }, snapshot);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const refresh = async () => {
    if (!status.configured || busy) return;
    setBusy(true);
    setError(null);
    try {
      const snapshot = await invoke<BinanceAccountSnapshot>("binance_account_snapshot");
      setAccountSnapshot(snapshot);
      onStatusChange({ configured: true, connected: true, message: "상품별 읽기 전용 계좌 연결을 확인했습니다." }, snapshot);
    } catch (reason) {
      setAccountSnapshot(null);
      setError(String(reason));
      onStatusChange({ ...status, connected: false }, null);
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (busy || !window.confirm("이 PC에 저장된 Binance API Key를 삭제할까요?")) return;
    setBusy(true);
    setError(null);
    try {
      const nextStatus = await invoke<BinanceConnectionStatus>("binance_delete_credentials");
      setAccountSnapshot(null);
      onStatusChange(nextStatus, null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const sections: Array<[string, BinanceAccountSection | undefined]> = [
    ["현물", accountSnapshot?.spot],
    ["USDⓈ-M", accountSnapshot?.usdM],
    ["COIN-M", accountSnapshot?.coinM],
  ];

  return <section className="settings-provider-connect settings-binance-connect" aria-labelledby="binance-connect-title">
    <header><div><span>BINANCE · READ ONLY</span><h3 id="binance-connect-title">현물·코인 선물 계좌 연결</h3></div><b>{status.connected ? "연결됨" : status.configured ? "저장됨" : "미연결"}</b></header>
    <p>{status.message} 현물·선물 거래와 출금 권한은 모두 비활성화하고, 읽기 권한 및 IP 제한만 적용한 키를 사용하세요.</p>
    {publicSnapshot && <div className="settings-binance-quotes" aria-label="Binance 공개 BTC 시세">
      {[publicSnapshot.spot, publicSnapshot.usdM, publicSnapshot.coinM].map((quote) => <article key={quote.market}><span>{quote.market}</span><strong>{quote.symbol}</strong><b>{formatValue(quote.price)}</b></article>)}
    </div>}
    {publicError && <p className="settings-public-feed-error" role="status">{publicError}</p>}
    <form onSubmit={save}>
      <label htmlFor="binance-api-key">API Key</label>
      <input id="binance-api-key" value={apiKey} onChange={(event) => setApiKey(event.currentTarget.value)} autoComplete="off" spellCheck={false} disabled={busy} />
      <label htmlFor="binance-secret-key">Secret Key</label>
      <input id="binance-secret-key" type="password" value={secretKey} onChange={(event) => setSecretKey(event.currentTarget.value)} autoComplete="new-password" spellCheck={false} disabled={busy} />
      <button className="settings-primary" type="submit" disabled={!apiKey || !secretKey || busy}>{busy ? "상품별 확인 중…" : "현물·선물 읽기 전용 연결"}</button>
    </form>
    {error && <div className="settings-error" role="alert"><strong>Binance 연결을 확인하지 못했습니다.</strong><span>{error}</span></div>}
    <div className="settings-provider-actions">
      <button type="button" onClick={() => void refresh()} disabled={!status.configured || busy}>상품별 계좌 조회</button>
      {status.configured && <button className="settings-danger" type="button" onClick={() => void remove()} disabled={busy}>Binance 연결 삭제</button>}
    </div>
    {accountSnapshot && <div className="settings-binance-sections" role="status">
      <small>{new Date(accountSnapshot.fetchedAtMs).toLocaleString("ko-KR")} · {accountSnapshot.permissionVerified ? accountSnapshot.permissionMessage : "권한 미검증"}</small>
      {sections.map(([label, section]) => <article className={section?.connected ? "is-connected" : "is-empty"} key={label}>
        <header><strong>{label}</strong><b>{section?.connected ? "조회 가능" : "미활성·권한 없음"}</b></header>
        <p>{section?.message}</p>
        {section?.connected && <span>잔고 {section.balances.length}종 · 포지션 {section.positions.length}건</span>}
      </article>)}
    </div>}
  </section>;
}
