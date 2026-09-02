import { type FormEvent, type KeyboardEvent, type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { EMPTY_MARKET_INDEX_SNAPSHOT, type MarketIndexSnapshot } from "./marketIndices";
import { KisPaperConnection, type PaperAccountSnapshot } from "./PaperTradingTerminal";
import { INTEGRATION_CATALOG, supportLabel } from "./integrationCatalog";
import { BinanceConnectionPanel, type BinanceAccountSnapshot, type BinanceConnectionStatus } from "./BinanceConnectionPanel";
import { TelegramConnectionPanel, type TelegramConnectionStatus } from "./TelegramConnectionPanel";
import { AiProviderConnections, type CodexConnectionStatus } from "./AiProviderConnections";
import { OfficialKrDataSettings } from "./OfficialKrDataSettings";
import { CONNECTION_LEGEND, connectionTone, type ConnectionTone } from "./connectionStatus";
import { summarizeConnectionRefresh, type ConnectionProbeResult, type ConnectionRefreshSummary } from "./connectionRefresh";
import { SocialLoginSettings } from "./SocialLoginSettings";

type TossConnectionStatus = {
  configured: boolean;
  connected: boolean;
  message: string;
};

type TossConnectionResult = {
  status: TossConnectionStatus;
  snapshot: MarketIndexSnapshot;
};

type KisConnectionStatus = {
  configured: boolean;
  connected: boolean;
  message: string;
};

type KisFuturesConnectionStatus = KisConnectionStatus & {
  provider: string;
  marketDataReady: boolean;
};

type UpbitConnectionStatus = {
  configured: boolean;
  connected: boolean;
  message: string;
};

type SecConnectionStatus = {
  configured: boolean;
  connected: boolean;
  message: string;
};

type UpbitAccountSnapshot = {
  provider: string;
  fetchedAtMs: number;
  readOnly: boolean;
  permissionVerified: boolean;
  accounts: Array<{ currency: string; balance: string; locked: string; avg_buy_price: string; unit_currency: string }>;
};

type TossAccountSnapshot = {
  provider: string;
  fetchedAtMs: number;
  readOnly: boolean;
  liveOrderEnabled: boolean;
  message: string;
  accounts: Array<{
    accountAlias: string;
    maskedAccountNo: string;
    accountType: string;
    holdings: {
      totalPurchaseAmount: { krw: string; usd?: string | null };
      marketValue: { amount: { krw: string; usd?: string | null } };
      profitLoss: { amount: { krw: string; usd?: string | null }; rate: string };
      items: Array<{ symbol: string; name: string; marketCountry: string; currency: string; quantity: string; lastPrice: string; averagePurchasePrice: string }>;
    };
  }>;
};

type TossSettingsDialogProps = {
  open: boolean;
  onClose: () => void;
  onSnapshot: (snapshot: MarketIndexSnapshot) => void;
  onPaperAccount: (snapshot: PaperAccountSnapshot) => void;
};

type AiProviderStatus = {
  provider: string;
  label: string;
  configured: boolean;
  connected: boolean;
};

type SafeInvokeResult<T> = { ok: true; value: T } | { ok: false };

const safeInvoke = async <T,>(command: string): Promise<SafeInvokeResult<T>> => {
  try {
    return { ok: true, value: await invoke<T>(command) };
  } catch {
    return { ok: false };
  }
};

function SettingsFold({ eyebrow, title, status, statusTone = "neutral", defaultOpen = false, children }: {
  eyebrow: string;
  title: string;
  status?: string;
  statusTone?: ConnectionTone;
  defaultOpen?: boolean;
  children: ReactNode;
}) {
  return <details className={`settings-fold is-${statusTone}`} open={defaultOpen}>
    <summary>
      <i className="settings-fold-status" aria-hidden="true" />
      <div><span>{eyebrow}</span><strong>{title}</strong></div>
      <small>{status}</small>
    </summary>
    <div className="settings-fold-content">{children}</div>
  </details>;
}

const EMPTY_STATUS: TossConnectionStatus = {
  configured: false,
  connected: false,
  message: "연결 상태를 확인하고 있습니다.",
};

const formatDecimalAmount = (value: string) => {
  const match = /^(-?)(\d+)(?:\.(\d+))?$/.exec(value);
  if (!match) return "확인 필요";
  const [, sign, integer, fraction] = match;
  const grouped = integer.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return `${sign}${grouped}${fraction ? `.${fraction}` : ""}`;
};

const parseFiniteDecimal = (value: string) => {
  if (!/^-?\d+(?:\.\d+)?$/.test(value)) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};

const formatHoldingMoney = (value: number | null, currency: string) => {
  if (value == null) return "확인 필요";
  return new Intl.NumberFormat("ko-KR", {
    style: "currency",
    currency: currency === "USD" ? "USD" : "KRW",
    maximumFractionDigits: currency === "USD" ? 2 : 0,
  }).format(value);
};

const holdingMetrics = (item: TossAccountSnapshot["accounts"][number]["holdings"]["items"][number]) => {
  const quantity = parseFiniteDecimal(item.quantity);
  const current = parseFiniteDecimal(item.lastPrice);
  const average = parseFiniteDecimal(item.averagePurchasePrice);
  if (quantity == null || current == null || average == null) return { marketValue: null, profitLoss: null, returnRate: null };
  const marketValue = quantity * current;
  const profitLoss = quantity * (current - average);
  const returnRate = average === 0 ? null : ((current - average) / average) * 100;
  return { marketValue, profitLoss, returnRate };
};

const groupHoldingsByCurrency = (items: TossAccountSnapshot["accounts"][number]["holdings"]["items"]) => items.reduce<Record<string, typeof items>>((groups, item) => {
  const currency = item.currency || (item.marketCountry === "US" ? "USD" : "KRW");
  (groups[currency] ??= []).push(item);
  return groups;
}, {});

const isTauriRuntime = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function TossSettingsDialog({ open, onClose, onSnapshot, onPaperAccount }: TossSettingsDialogProps) {
  const [status, setStatus] = useState<TossConnectionStatus>(EMPTY_STATUS);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [accountSnapshot, setAccountSnapshot] = useState<TossAccountSnapshot | null>(null);
  const [paperAccount, setPaperAccount] = useState<PaperAccountSnapshot | null>(null);
  const [codexStatus, setCodexStatus] = useState<CodexConnectionStatus | null>(null);
  const [kisStatus, setKisStatus] = useState<KisConnectionStatus | null>(null);
  const [kisFuturesStatus, setKisFuturesStatus] = useState<KisFuturesConnectionStatus | null>(null);
  const [upbitStatus, setUpbitStatus] = useState<UpbitConnectionStatus>({ configured: false, connected: false, message: "상태 확인 중" });
  const [secStatus, setSecStatus] = useState<SecConnectionStatus>({ configured: false, connected: false, message: "상태 확인 중" });
  const [secContact, setSecContact] = useState("");
  const [secBusy, setSecBusy] = useState(false);
  const [secError, setSecError] = useState<string | null>(null);
  const [upbitAccessKey, setUpbitAccessKey] = useState("");
  const [upbitSecretKey, setUpbitSecretKey] = useState("");
  const [upbitSnapshot, setUpbitSnapshot] = useState<UpbitAccountSnapshot | null>(null);
  const [upbitBusy, setUpbitBusy] = useState(false);
  const [upbitError, setUpbitError] = useState<string | null>(null);
  const [binanceStatus, setBinanceStatus] = useState<BinanceConnectionStatus>({ configured: false, connected: false, message: "상태 확인 중" });
  const [binanceSnapshot, setBinanceSnapshot] = useState<BinanceAccountSnapshot | null>(null);
  const [telegramStatus, setTelegramStatus] = useState<TelegramConnectionStatus>({ configured: false, sessionStored: false, authorized: false, selectedChannelCount: 0, message: "상태 확인 중" });
  const [accountBusy, setAccountBusy] = useState(false);
  const [connectionRefreshBusy, setConnectionRefreshBusy] = useState(false);
  const [connectionRefreshSummary, setConnectionRefreshSummary] = useState<ConnectionRefreshSummary | null>(null);
  const [connectionRefreshSource, setConnectionRefreshSource] = useState<"automatic" | "manual" | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const clientIdRef = useRef<HTMLInputElement>(null);
  const mountedRef = useRef(true);
  const connectionRefreshBusyRef = useRef(false);
  const automaticRefreshStartedRef = useRef(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const refreshAllConnections = useCallback(async (source: "automatic" | "manual") => {
    if (connectionRefreshBusyRef.current) return;
    if (!isTauriRuntime) {
      setConnectionRefreshSource(source);
      setConnectionRefreshSummary(summarizeConnectionRefresh([
        { id: "desktop", label: "Investa 데스크톱", state: "failed" },
      ], Date.now()));
      return;
    }

    connectionRefreshBusyRef.current = true;
    setConnectionRefreshBusy(true);
    setConnectionRefreshSource(source);

    const [toss, codex, kis, kisFutures, upbit, sec, binance, telegram, paper, aiProviders] = await Promise.all([
      safeInvoke<TossConnectionStatus>("toss_connection_status"),
      safeInvoke<CodexConnectionStatus>("codex_status"),
      safeInvoke<KisConnectionStatus>("kis_paper_config_status"),
      safeInvoke<KisFuturesConnectionStatus>("kis_futures_connection_status"),
      safeInvoke<UpbitConnectionStatus>("upbit_connection_status"),
      safeInvoke<SecConnectionStatus>("sec_connection_status"),
      safeInvoke<BinanceConnectionStatus>("binance_connection_status"),
      safeInvoke<TelegramConnectionStatus>("telegram_connection_status"),
      safeInvoke<PaperAccountSnapshot>("paper_account_status"),
      safeInvoke<AiProviderStatus[]>("ai_provider_statuses"),
    ]);

    if (mountedRef.current) {
      if (toss.ok) setStatus(toss.value);
      if (codex.ok) setCodexStatus(codex.value);
      if (kis.ok) setKisStatus(kis.value);
      if (kisFutures.ok) setKisFuturesStatus(kisFutures.value);
      if (upbit.ok) setUpbitStatus(upbit.value);
      if (sec.ok) setSecStatus(sec.value);
      if (binance.ok) setBinanceStatus(binance.value);
      if (telegram.ok) setTelegramStatus(telegram.value);
      if (paper.ok) {
        setPaperAccount(paper.value);
        onPaperAccount(paper.value);
      }
    }

    const probes: Array<Promise<ConnectionProbeResult>> = [];

    if (!toss.ok) {
      probes.push(Promise.resolve({ id: "toss", label: "토스증권", state: "failed" }));
    } else if (!toss.value.configured) {
      probes.push(Promise.resolve({ id: "toss", label: "토스증권", state: "disconnected" }));
    } else {
      probes.push((async () => {
        const [market, account] = await Promise.all([
          safeInvoke<MarketIndexSnapshot>("market_indices_snapshot"),
          safeInvoke<TossAccountSnapshot>("toss_account_snapshot"),
        ]);
        if (mountedRef.current) {
          if (market.ok) onSnapshot(market.value);
          if (account.ok) setAccountSnapshot(account.value);
          setStatus({
            configured: true,
            connected: account.ok,
            message: account.ok ? "읽기 전용 시세·계좌 조회를 확인했습니다." : "자격정보는 저장됐지만 계좌 조회를 확인하지 못했습니다.",
          });
        }
        return { id: "toss", label: "토스증권", state: account.ok ? "connected" : "attention" };
      })());
    }

    if (!upbit.ok) {
      probes.push(Promise.resolve({ id: "upbit", label: "Upbit", state: "failed" }));
    } else if (!upbit.value.configured) {
      probes.push(Promise.resolve({ id: "upbit", label: "Upbit", state: "disconnected" }));
    } else {
      probes.push((async () => {
        const account = await safeInvoke<UpbitAccountSnapshot>("upbit_account_snapshot");
        if (mountedRef.current) {
          setUpbitSnapshot(account.ok ? account.value : null);
          setUpbitStatus({ configured: true, connected: account.ok, message: account.ok ? "계좌 조회 성공 · Upbit 관리 화면에서 주문·출금 권한 비활성화 확인 필요" : "키는 저장됐지만 계좌 조회를 확인하지 못했습니다." });
        }
        return { id: "upbit", label: "Upbit", state: account.ok && account.value.permissionVerified ? "connected" : "attention" };
      })());
    }

    if (!binance.ok) {
      probes.push(Promise.resolve({ id: "binance", label: "Binance", state: "failed" }));
    } else if (!binance.value.configured) {
      probes.push(Promise.resolve({ id: "binance", label: "Binance", state: "disconnected" }));
    } else {
      probes.push((async () => {
        const account = await safeInvoke<BinanceAccountSnapshot>("binance_account_snapshot");
        const connected = account.ok && (account.value.spot.connected || account.value.usdM.connected || account.value.coinM.connected);
        if (mountedRef.current) {
          setBinanceSnapshot(account.ok ? account.value : null);
          setBinanceStatus({ configured: true, connected, message: connected ? "상품별 읽기 전용 계좌 연결을 확인했습니다." : "키는 저장됐지만 활성 계좌 조회를 확인하지 못했습니다." });
        }
        return { id: "binance", label: "Binance", state: connected ? "connected" : "attention" };
      })());
    }

    if (!kis.ok) {
      probes.push(Promise.resolve({ id: "kis", label: "KIS 모의투자", state: "failed" }));
    } else if (!kis.value.configured) {
      probes.push(Promise.resolve({ id: "kis", label: "KIS 모의투자", state: "disconnected" }));
    } else {
      probes.push((async () => {
        const account = await safeInvoke<unknown>("kis_paper_account_snapshot");
        if (mountedRef.current) setKisStatus({ configured: true, connected: account.ok, message: account.ok ? "KIS 모의계좌 조회를 확인했습니다." : "정보는 저장됐지만 모의계좌 조회를 확인하지 못했습니다." });
        return { id: "kis", label: "KIS 모의투자", state: account.ok ? "connected" : "attention" };
      })());
    }

    if (!sec.ok) {
      probes.push(Promise.resolve({ id: "sec", label: "SEC", state: "failed" }));
    } else if (!sec.value.configured) {
      probes.push(Promise.resolve({ id: "sec", label: "SEC", state: "disconnected" }));
    } else {
      probes.push((async () => {
        const result = await safeInvoke<SecConnectionStatus>("sec_connection_probe");
        if (mountedRef.current) setSecStatus(result.ok ? result.value : { configured: true, connected: false, message: "연락처는 저장됐지만 SEC 응답을 확인하지 못했습니다." });
        return { id: "sec", label: "SEC", state: result.ok ? "connected" : "attention" };
      })());
    }

    probes.push(Promise.resolve(!telegram.ok
      ? { id: "telegram", label: "Telegram", state: "failed" }
      : { id: "telegram", label: "Telegram", state: telegram.value.authorized ? "connected" : telegram.value.configured ? "attention" : "disconnected" }));
    probes.push(Promise.resolve(!codex.ok
      ? { id: "codex", label: "Codex", state: "failed" }
      : { id: "codex", label: "Codex", state: codex.value.connected ? "connected" : codex.value.available ? "attention" : "disconnected" }));

    if (!aiProviders.ok) {
      probes.push(Promise.resolve({ id: "external-ai", label: "외부 AI", state: "failed" }));
    } else {
      for (const provider of aiProviders.value) {
        probes.push(Promise.resolve({
          id: provider.provider,
          label: provider.label,
          state: provider.connected ? "connected" : provider.configured ? "attention" : "disconnected",
        }));
      }
    }

    const results = await Promise.all(probes);
    if (mountedRef.current) setConnectionRefreshSummary(summarizeConnectionRefresh(results, Date.now()));
    connectionRefreshBusyRef.current = false;
    if (mountedRef.current) setConnectionRefreshBusy(false);
  }, [onPaperAccount, onSnapshot]);

  useEffect(() => {
    if (automaticRefreshStartedRef.current) return;
    automaticRefreshStartedRef.current = true;
    void refreshAllConnections("automatic");
  }, [refreshAllConnections]);

  useEffect(() => {
    if (!open) {
      setClientId("");
      setClientSecret("");
      setError(null);
      setConfirmingDelete(false);
      setAccountSnapshot(null);
      setUpbitAccessKey("");
      setUpbitSecretKey("");
      setUpbitError(null);
      setSecContact("");
      setSecError(null);
      return;
    }
    setError(null);
    setConfirmingDelete(false);
    if (!isTauriRuntime) {
      setStatus({ configured: false, connected: false, message: "Investa 데스크톱 앱에서 연결할 수 있습니다." });
      window.setTimeout(() => {
        dialogRef.current?.scrollTo({ top: 0 });
        closeButtonRef.current?.focus();
      }, 0);
      return;
    }
    window.setTimeout(() => {
      dialogRef.current?.scrollTo({ top: 0 });
      closeButtonRef.current?.focus();
    }, 0);
  }, [open]);

  if (!open) return null;

  const handleDialogKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape" && !busy) {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = dialogRef.current?.querySelectorAll<HTMLElement>(
      "button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex='-1'])",
    );
    if (!focusable?.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const handleSave = async (event: FormEvent) => {
    event.preventDefault();
    if (!isTauriRuntime || !clientId || !clientSecret || busy) return;
    setBusy(true);
    setError(null);
    setConfirmingDelete(false);
    try {
      const result = await invoke<TossConnectionResult>("toss_save_credentials", {
        request: { clientId, clientSecret },
      });
      setStatus(result.status);
      onSnapshot(result.snapshot);
      setClientId("");
      setClientSecret("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async () => {
    if (!confirmingDelete) {
      setConfirmingDelete(true);
      setError(null);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const nextStatus = await invoke<TossConnectionStatus>("toss_delete_credentials");
      setStatus(nextStatus);
      setClientId("");
      setClientSecret("");
      setConfirmingDelete(false);
      setAccountSnapshot(null);
      onSnapshot(EMPTY_MARKET_INDEX_SNAPSHOT);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const handleAccountSync = async () => {
    if (!status.configured || accountBusy) return;
    setAccountBusy(true);
    setError(null);
    try {
      const snapshot = await invoke<TossAccountSnapshot>("toss_account_snapshot");
      setAccountSnapshot(snapshot);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setAccountBusy(false);
    }
  };

  const statusClass = status.connected ? "is-connected" : status.configured ? "is-configured" : "is-empty";

  const handleUpbitSave = async (event: FormEvent) => {
    event.preventDefault();
    if (!isTauriRuntime || upbitBusy || !upbitAccessKey || !upbitSecretKey) return;
    setUpbitBusy(true);
    setUpbitError(null);
    try {
      const nextStatus = await invoke<UpbitConnectionStatus>("upbit_save_credentials", { request: { accessKey: upbitAccessKey, secretKey: upbitSecretKey } });
      setUpbitStatus(nextStatus);
      setUpbitAccessKey("");
      setUpbitSecretKey("");
      setUpbitSnapshot(await invoke<UpbitAccountSnapshot>("upbit_account_snapshot"));
    } catch (reason) {
      setUpbitError(String(reason));
    } finally {
      setUpbitBusy(false);
    }
  };

  const handleUpbitSync = async () => {
    if (!upbitStatus.configured || upbitBusy) return;
    setUpbitBusy(true);
    setUpbitError(null);
    try {
      const snapshot = await invoke<UpbitAccountSnapshot>("upbit_account_snapshot");
      setUpbitSnapshot(snapshot);
      setUpbitStatus({ configured: true, connected: true, message: "계좌 조회 성공 · Upbit 관리 화면에서 주문·출금 권한 비활성화 확인 필요" });
    } catch (reason) {
      setUpbitSnapshot(null);
      setUpbitStatus((current) => ({ ...current, connected: false }));
      setUpbitError(String(reason));
    } finally {
      setUpbitBusy(false);
    }
  };

  const handleUpbitDelete = async () => {
    if (upbitBusy || !window.confirm("이 PC에 저장된 Upbit API 키를 삭제할까요?")) return;
    setUpbitBusy(true);
    setUpbitError(null);
    try {
      setUpbitStatus(await invoke<UpbitConnectionStatus>("upbit_delete_credentials"));
      setUpbitSnapshot(null);
      setUpbitAccessKey("");
      setUpbitSecretKey("");
    } catch (reason) {
      setUpbitError(String(reason));
    } finally {
      setUpbitBusy(false);
    }
  };

  const handleSecSave = async (event: FormEvent) => {
    event.preventDefault();
    if (!isTauriRuntime || secBusy || !secContact) return;
    setSecBusy(true);
    setSecError(null);
    try {
      const nextStatus = await invoke<SecConnectionStatus>("sec_save_contact", { request: { contactEmail: secContact } });
      setSecStatus(nextStatus);
      setSecContact("");
    } catch (reason) {
      setSecError(String(reason));
    } finally {
      setSecBusy(false);
    }
  };

  const handleSecDelete = async () => {
    if (secBusy || !window.confirm("이 PC에 저장된 SEC 요청 연락처를 삭제할까요?")) return;
    setSecBusy(true);
    setSecError(null);
    try {
      setSecStatus(await invoke<SecConnectionStatus>("sec_delete_contact"));
      setSecContact("");
    } catch (reason) {
      setSecError(String(reason));
    } finally {
      setSecBusy(false);
    }
  };

  return (
    <div className="settings-backdrop">
      <div
        className="settings-dialog"
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
        aria-describedby="settings-description"
        onKeyDown={handleDialogKeyDown}
      >
        <header className="settings-header">
          <div><span>LOCAL CONNECTION HUB</span><h2 id="settings-title">연결·계좌 설정</h2></div>
          <button ref={closeButtonRef} className="icon-button" type="button" onClick={onClose} disabled={busy} aria-label="설정 닫기">×</button>
        </header>

        <SettingsFold eyebrow="CONNECTION STATUS" title="시장·계좌 연결 상태" status="실전 주문 전체 잠금" defaultOpen>
        <section className="settings-integration-overview" aria-labelledby="integration-overview-title">
          <header><div><span>CONNECTION STATUS</span><h3 id="integration-overview-title">시장·계좌 연결 상태</h3></div><small>실전 주문은 전체 잠금</small></header>
          <div className="settings-connection-refresh">
            <div>
              <strong>{connectionRefreshBusy ? "전체 연결을 조회하고 있습니다." : connectionRefreshSummary ? `${connectionRefreshSummary.total}개 공급자 조회 완료` : "로그인 후 자동으로 연결을 조회합니다."}</strong>
              <span role="status" aria-live="polite">
                {connectionRefreshBusy
                  ? "저장된 자격정보로 읽기 전용 상태만 확인합니다. 주문·출금·유료 AI 분석은 실행하지 않습니다."
                  : connectionRefreshSummary
                    ? `연결 ${connectionRefreshSummary.connected} · 확인 필요 ${connectionRefreshSummary.attention} · 미연결 ${connectionRefreshSummary.disconnected} · 실패 ${connectionRefreshSummary.failed} · ${new Date(connectionRefreshSummary.completedAtMs).toLocaleString("ko-KR")} ${connectionRefreshSource === "automatic" ? "자동 조회" : "수동 조회"}`
                    : "설정을 열지 않아도 로그인 직후 한 번 확인합니다."}
              </span>
            </div>
            <button type="button" onClick={() => void refreshAllConnections("manual")} disabled={connectionRefreshBusy}>
              {connectionRefreshBusy ? "전체 연결 조회 중…" : "전체 연결 조회"}
            </button>
          </div>
          <div className="settings-integration-grid">
            <article className={`is-${connectionTone(status.connected, status.configured)}`}><i aria-hidden="true" /><div><strong>국장</strong><span>{status.connected ? "토스 시세·계좌 조회 지원" : status.configured ? "토스 저장됨 · 계좌 확인 필요" : "시세·계좌 미연결"}</span></div><b>{kisStatus?.connected ? "KIS 모의 연결" : kisStatus?.configured ? "KIS 저장됨" : "KIS 미연결"}</b></article>
            <article className={`is-${connectionTone(status.connected, status.configured || secStatus.configured)}`}><i aria-hidden="true" /><div><strong>미장</strong><span>{status.connected ? `토스 시세${secStatus.connected ? " · SEC 재무" : ""}` : secStatus.configured ? "SEC 정보 저장 · 시세 확인 필요" : "시세·재무 미연결"}</span></div><b>주문 어댑터 필요</b></article>
            <article className={`is-${connectionTone(upbitStatus.connected && Boolean(upbitSnapshot?.permissionVerified), upbitStatus.configured)}`}><i aria-hidden="true" /><div><strong>코인</strong><span>Upbit 공개 시세 사용 가능</span></div><b>{upbitStatus.connected ? "계좌 조회 · 권한 확인 필요" : upbitStatus.configured ? "키 저장됨 · 확인 필요" : "개인 계좌 미연결"}</b></article>
            <article className={`is-${connectionTone(Boolean(kisFuturesStatus?.marketDataReady), Boolean(kisFuturesStatus?.configured))}`}><i aria-hidden="true" /><div><strong>증권 선물</strong><span>{kisFuturesStatus?.marketDataReady ? "KIS 공식 국내선물 일봉 준비" : "내부 sandbox 사용 가능"}</span></div><b>{kisFuturesStatus?.marketDataReady ? "읽기 전용 준비" : kisFuturesStatus?.configured ? "정보 저장 · 확인 필요" : "KIS 미연결"}</b></article>
            <article className={`is-${connectionTone(Boolean(binanceSnapshot?.usdM.connected || binanceSnapshot?.coinM.connected), binanceStatus.configured)}`}><i aria-hidden="true" /><div><strong>코인 선물</strong><span>Binance USDⓈ-M·COIN-M</span></div><b>{binanceSnapshot?.usdM.connected || binanceSnapshot?.coinM.connected ? "읽기 전용 연결" : binanceStatus.configured ? "키 저장됨 · 확인 필요" : "개인 계좌 미연결"}</b></article>
            <article className={secStatus.connected ? "is-connected" : secStatus.configured ? "is-partial" : "is-empty"}><i aria-hidden="true" /><div><strong>SEC 공시·재무</strong><span>미국 공식 Company Facts·Submissions</span></div><b>{secStatus.connected ? "연결 완료" : secStatus.configured ? "연락처 저장됨" : "미연결"}</b></article>
            <article className={telegramStatus.authorized ? "is-connected" : telegramStatus.configured ? "is-partial" : "is-empty"}><i aria-hidden="true" /><div><strong>Telegram 뉴스</strong><span>{telegramStatus.authorized ? `읽기 전용 세션 · 선택 채널 ${telegramStatus.selectedChannelCount}개` : "선택 방송 채널 수집"}</span></div><b>{telegramStatus.authorized ? "연결 완료" : telegramStatus.configured ? "로그인 확인 필요" : "미연결"}</b></article>
          </div>
          <div className="settings-status-legend" aria-label="연결 상태 색상 안내">
            {CONNECTION_LEGEND.map((item) => <span className={`is-${item.tone}`} key={item.tone}><i aria-hidden="true" /><strong>{item.label}</strong><small>{item.description}</small></span>)}
          </div>
        </section>
        </SettingsFold>

        <SettingsFold eyebrow="SEC · OFFICIAL FUNDAMENTALS" title="미장 공식 재무 연결" status={secStatus.connected ? "연결됨" : secStatus.configured ? "저장됨" : "미연결"} statusTone={connectionTone(secStatus.connected, secStatus.configured)}>
        <section className="settings-provider-connect" aria-labelledby="sec-connect-title">
          <header><div><span>SEC · OFFICIAL FUNDAMENTALS</span><h3 id="sec-connect-title">미장 공식 재무 연결</h3></div><b>{secStatus.connected ? "연결됨" : secStatus.configured ? "저장됨" : "미연결"}</b></header>
          <p>{secStatus.message} API 키는 없지만 SEC 정책에 따라 요청 User-Agent에 연락 이메일을 포함합니다. 이메일은 Windows 자격 증명 관리자에만 저장됩니다.</p>
          <form onSubmit={handleSecSave}>
            <label htmlFor="sec-contact-email">SEC 요청 연락 이메일</label>
            <input id="sec-contact-email" type="email" value={secContact} onChange={(event) => setSecContact(event.currentTarget.value)} autoComplete="email" spellCheck={false} disabled={secBusy} />
            <button className="settings-primary" type="submit" disabled={!secContact || secBusy}>{secBusy ? "연결 확인 중…" : "SEC 재무 연결 확인"}</button>
          </form>
          {secError && <div className="settings-error" role="alert"><strong>SEC 연결을 확인하지 못했습니다.</strong><span>{secError}</span></div>}
          {secStatus.configured && <div className="settings-provider-actions"><button className="settings-danger" type="button" onClick={() => void handleSecDelete()} disabled={secBusy}>SEC 연락처 삭제</button></div>}
          <small>현재 범위: 미국 상장사의 10-K·10-Q·20-F·40-F·6-K Company Facts. 국장 재무와 뉴스·수급은 아직 연결하지 않습니다.</small>
        </section>
        </SettingsFold>

        <SettingsFold eyebrow="TELEGRAM · READ ONLY" title="투자 뉴스 채널 수집" status={telegramStatus.authorized ? `연결됨 · ${telegramStatus.selectedChannelCount}개 채널` : telegramStatus.configured ? "로그인 확인 필요" : "미연결"} statusTone={connectionTone(telegramStatus.authorized, telegramStatus.configured)}>
        <TelegramConnectionPanel open={open} onStatusChange={setTelegramStatus} />
        </SettingsFold>

        <div className={`settings-status ${statusClass}`} role="status">
          <i aria-hidden="true" />
          <div><strong>토스증권 · {status.connected ? "연결됨" : status.configured ? "저장됨" : "미설정"}</strong><p>{status.message}</p></div>
        </div>

        <SettingsFold eyebrow="ADAPTER CATALOG" title="공급자 지원 범위" status="확장 가능">
        <section className="settings-scope" id="settings-description">
          <span>공급자 어댑터 방식</span>
          <p>계좌번호만으로 모든 증권사를 연결할 수는 없습니다. 각 증권사·거래소의 공식 API 어댑터가 있어야 하며, 미지원 공급자는 연결된 것처럼 표시하지 않습니다. 배포 사용자는 본인이 발급한 자격정보만 이 PC에서 입력합니다.</p>
        </section>

        <section className="settings-adapter-catalog" aria-labelledby="adapter-catalog-title">
          <header><div><span>ADAPTER CATALOG</span><h3 id="adapter-catalog-title">지원 공급자</h3></div><small>확장 가능</small></header>
          <div>{INTEGRATION_CATALOG.filter((item) => item.kind !== "ai").map((item) => <article key={item.id}><strong>{item.name}</strong><span>{item.summary}</span><b>{supportLabel[item.support]}</b></article>)}</div>
        </section>
        </SettingsFold>

        <SettingsFold eyebrow="UPBIT · READ ONLY" title="코인 개인계좌 연결" status={upbitStatus.connected ? "연결됨" : upbitStatus.configured ? "저장됨" : "미연결"} statusTone={connectionTone(upbitStatus.connected, upbitStatus.configured)}>
        <section className="settings-provider-connect" aria-labelledby="upbit-connect-title">
          <header><div><span>UPBIT · READ ONLY</span><h3 id="upbit-connect-title">코인 개인계좌 연결</h3></div><b>{upbitStatus.connected ? "연결됨" : upbitStatus.configured ? "저장됨" : "미연결"}</b></header>
          <p>{upbitStatus.message} 조회 권한만 허용한 API 키를 사용하고 주문·출금 권한은 부여하지 마세요.</p>
          <form onSubmit={handleUpbitSave}>
            <label htmlFor="upbit-access-key">Access Key</label>
            <input id="upbit-access-key" value={upbitAccessKey} onChange={(event) => setUpbitAccessKey(event.currentTarget.value)} autoComplete="off" spellCheck={false} disabled={upbitBusy} />
            <label htmlFor="upbit-secret-key">Secret Key</label>
            <input id="upbit-secret-key" type="password" value={upbitSecretKey} onChange={(event) => setUpbitSecretKey(event.currentTarget.value)} autoComplete="new-password" spellCheck={false} disabled={upbitBusy} />
            <button className="settings-primary" type="submit" disabled={!upbitAccessKey || !upbitSecretKey || upbitBusy}>{upbitBusy ? "확인 중…" : "읽기 전용으로 연결"}</button>
          </form>
          {upbitError && <div className="settings-error" role="alert"><strong>Upbit 연결을 확인하지 못했습니다.</strong><span>{upbitError}</span></div>}
          <div className="settings-provider-actions">
            <button className="settings-balance-button" type="button" onClick={() => void handleUpbitSync()} disabled={!upbitStatus.configured || upbitBusy}>계좌 잔고 조회</button>
            {upbitStatus.configured && <button className="settings-danger" type="button" onClick={() => void handleUpbitDelete()} disabled={upbitBusy}>Upbit 연결 삭제</button>}
          </div>
          {upbitSnapshot && <div className="settings-coin-balances" role="status">
            <small>{new Date(upbitSnapshot.fetchedAtMs).toLocaleString("ko-KR")} · 계좌 조회 성공 · 주문·출금 권한 미검증</small>
            {upbitSnapshot.accounts.length === 0 ? <p>보유 잔고가 없습니다.</p> : upbitSnapshot.accounts.map((account) => <article key={account.currency}><strong>{account.currency}</strong><span>사용 가능 {formatDecimalAmount(account.balance)}</span><span>주문 잠김 {formatDecimalAmount(account.locked)}</span></article>)}
          </div>}
        </section>
        </SettingsFold>

        <SettingsFold eyebrow="BINANCE · READ ONLY" title="현물·코인 선물 계좌 연결" status={binanceStatus.connected ? "연결됨" : binanceStatus.configured ? "저장됨" : "미연결"} statusTone={connectionTone(binanceStatus.connected, binanceStatus.configured)}>
        <BinanceConnectionPanel status={binanceStatus} refreshedSnapshot={binanceSnapshot} onStatusChange={(nextStatus, snapshot) => { setBinanceStatus(nextStatus); if (snapshot !== undefined) setBinanceSnapshot(snapshot); }} />
        </SettingsFold>

        <SettingsFold eyebrow="KIS VIRTUAL TRADING" title="KIS 모의계좌 연결" status={kisStatus?.connected ? "연결됨" : kisStatus?.configured ? "저장됨" : "미연결"} statusTone={connectionTone(Boolean(kisStatus?.connected), Boolean(kisStatus?.configured))}>
        <p className="settings-provider-note">{kisFuturesStatus?.message ?? "KIS 증권선물 읽기 전용 시세 상태를 확인하고 있습니다."} 현재 만기 계약코드를 직접 지정하며 근월물·정산가를 추정하지 않습니다.</p>
        <KisPaperConnection />
        </SettingsFold>

        <SettingsFold eyebrow="KR STOCK · READ ONLY" title="국장 계좌·내부 모의원장" status={status.connected ? "연결됨" : status.configured ? "저장됨" : "미연결"} statusTone={connectionTone(status.connected, status.configured)}>
        <section className="settings-account" aria-labelledby="settings-account-title">
          <header><div><span>KR STOCK · READ ONLY</span><h3 id="settings-account-title">국장 토스증권 계좌·내부 모의원장</h3></div><button className="settings-balance-button" type="button" onClick={() => void handleAccountSync()} disabled={!status.configured || accountBusy}>{accountBusy ? "국장 계좌 조회 중…" : "국장 계좌 잔고 조회"}</button></header>
          <div className="settings-paper-status"><span>내부 모의계좌</span><strong>{paperAccount ? `₩${paperAccount.account.cashMinor.toLocaleString("ko-KR")}` : "확인 중"}</strong><small>{paperAccount ? `포지션 ${Object.keys(paperAccount.account.positions).length}개 · 원장 ${paperAccount.account.eventCount}건` : "1억원 KRW 계좌를 준비합니다."}</small></div>
          {accountSnapshot && <div className="settings-account-results" role="status">
            <p>{accountSnapshot.message}</p>
            {accountSnapshot.accounts.map((account, accountIndex) => {
              const holdingsByCurrency = groupHoldingsByCurrency(account.holdings.items);
              return <details className="settings-holdings-account" key={`${account.maskedAccountNo}-${account.accountAlias}`} open={accountIndex === 0}>
                <summary>
                  <div><strong>{account.accountAlias}</strong><span>{account.maskedAccountNo} · {account.accountType}</span></div>
                  <dl><div><dt>평가금액</dt><dd>₩{formatDecimalAmount(account.holdings.marketValue.amount.krw)}</dd></div><div><dt>평가손익</dt><dd>₩{formatDecimalAmount(account.holdings.profitLoss.amount.krw)}</dd></div><div><dt>보유종목</dt><dd>{account.holdings.items.length}개</dd></div></dl>
                </summary>
                <div className="settings-holdings-groups">
                  {Object.entries(holdingsByCurrency).map(([currency, items]) => <section key={currency} aria-label={`${currency} 보유자산`}>
                    <header><strong>{currency === "KRW" ? "국내주식 · KRW" : currency === "USD" ? "미국주식 · USD" : `${currency} 보유자산`}</strong><span>{items?.length ?? 0}종목</span></header>
                    <div className="settings-holdings-table-wrap">
                      <table>
                        <thead><tr><th>종목</th><th>수량</th><th>평균단가</th><th>현재가</th><th>평가금액</th><th>평가손익</th><th>수익률</th></tr></thead>
                        <tbody>{items?.map((item) => {
                          const metrics = holdingMetrics(item);
                          return <tr key={`${item.marketCountry}-${item.symbol}`}><th scope="row"><strong>{item.name}</strong><span>{item.symbol}</span></th><td>{formatDecimalAmount(item.quantity)}</td><td>{formatHoldingMoney(parseFiniteDecimal(item.averagePurchasePrice), currency)}</td><td>{formatHoldingMoney(parseFiniteDecimal(item.lastPrice), currency)}</td><td>{formatHoldingMoney(metrics.marketValue, currency)}</td><td className={metrics.profitLoss == null ? "" : metrics.profitLoss >= 0 ? "is-positive" : "is-negative"}>{formatHoldingMoney(metrics.profitLoss, currency)}</td><td className={metrics.returnRate == null ? "" : metrics.returnRate >= 0 ? "is-positive" : "is-negative"}>{metrics.returnRate == null ? "확인 필요" : `${metrics.returnRate >= 0 ? "+" : ""}${metrics.returnRate.toFixed(2)}%`}</td></tr>;
                        })}</tbody>
                      </table>
                    </div>
                  </section>)}
                  {account.holdings.items.length === 0 && <p className="settings-holdings-empty">보유 중인 주식이 없습니다.</p>}
                </div>
              </details>;
            })}
            <small>{new Date(accountSnapshot.fetchedAtMs).toLocaleString("ko-KR")} · 실주문 {accountSnapshot.liveOrderEnabled ? "활성" : "잠금"}</small>
          </div>}
          <p className="settings-paper-moved">모의주문·차트·보조지표·주문 취소는 왼쪽 <strong>모의</strong> 메뉴에서 관리합니다.</p>
        </section>
        </SettingsFold>

        <SettingsFold eyebrow="TOSS OPEN API" title="토스증권 자격정보" status={status.connected ? "연결됨" : status.configured ? "이 PC에 저장됨" : "미설정"} statusTone={connectionTone(status.connected, status.configured)}>
        <form className="settings-form" onSubmit={handleSave}>
          <label htmlFor="toss-client-id">Client ID</label>
          <input
            id="toss-client-id"
            ref={clientIdRef}
            value={clientId}
            onChange={(event) => setClientId(event.currentTarget.value)}
            autoComplete="off"
            spellCheck={false}
            disabled={busy}
          />
          <label htmlFor="toss-client-secret">Client Secret</label>
          <input
            id="toss-client-secret"
            type="password"
            value={clientSecret}
            onChange={(event) => setClientSecret(event.currentTarget.value)}
            autoComplete="new-password"
            spellCheck={false}
            disabled={busy}
          />
          <p className="settings-secret-note">입력값은 React 상태에 잠시만 존재하며, 연결 확인 후 Windows 자격 증명 관리자에 저장됩니다.</p>
          {error && <div className="settings-error" role="alert"><strong>연결하지 못했습니다.</strong><span>{error}</span></div>}
          <button className="settings-primary" type="submit" disabled={!isTauriRuntime || !clientId || !clientSecret || busy}>
            {busy ? "토스증권 연결 확인 중…" : "저장하고 연결 확인"}
          </button>
        </form>
        </SettingsFold>

        <footer className="settings-footer">
          {confirmingDelete && <p role="alert">저장된 Client ID와 Client Secret을 이 PC에서 삭제합니다.</p>}
          <div>
            {status.configured && <button className="settings-danger" type="button" onClick={handleDelete} disabled={busy}>
              {confirmingDelete ? "연결 정보 삭제 확정" : "연결 정보 삭제"}
            </button>}
            {confirmingDelete && <button type="button" onClick={() => setConfirmingDelete(false)} disabled={busy}>취소</button>}
          </div>
          <SettingsFold eyebrow="LOGIN PROVIDERS" title="로그인 공급자" status="GitHub 기본 · Google 선택 · Apple 준비 중" statusTone="partial">
            <SocialLoginSettings open={open} />
          </SettingsFold>
          <SettingsFold eyebrow="AI PROVIDERS" title="분석 모델 연결" status={codexStatus?.connected ? "Codex 연결됨" : "사용자 설정 필요"} statusTone={connectionTone(Boolean(codexStatus?.connected))}>
          <AiProviderConnections open={open} codexStatus={codexStatus} />
          <OfficialKrDataSettings open={open} />
          </SettingsFold>
        </footer>
      </div>
    </div>
  );
}
