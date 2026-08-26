import { type FormEvent, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type TelegramConnectionStatus = {
  configured: boolean;
  sessionStored: boolean;
  authorized: boolean;
  selectedChannelCount: number;
  message: string;
};

type TelegramLoginState = {
  stage: "code" | "password" | "authorized";
  passwordHint?: string | null;
  message: string;
};

type TelegramChannel = {
  peerId: number;
  title: string;
  username?: string | null;
  selected: boolean;
};

type TelegramSyncResult = {
  selectedChannelCount: number;
  fetchedMessageCount: number;
  insertedRevisionCount: number;
  syncedAtMs: number;
  message: string;
  channels: Array<{
    peerId: number;
    title: string;
    status: "synced" | "failed";
    fetchedMessageCount: number;
    insertedRevisionCount: number;
    error?: string | null;
  }>;
};

type TelegramConnectionPanelProps = {
  open: boolean;
};

const EMPTY_STATUS: TelegramConnectionStatus = {
  configured: false,
  sessionStored: false,
  authorized: false,
  selectedChannelCount: 0,
  message: "연결 상태를 확인하고 있습니다.",
};

export function TelegramConnectionPanel({ open }: TelegramConnectionPanelProps) {
  const [status, setStatus] = useState<TelegramConnectionStatus>(EMPTY_STATUS);
  const [apiId, setApiId] = useState("");
  const [apiHash, setApiHash] = useState("");
  const [phone, setPhone] = useState("");
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const [loginState, setLoginState] = useState<TelegramLoginState | null>(null);
  const [channels, setChannels] = useState<TelegramChannel[]>([]);
  const [syncResult, setSyncResult] = useState<TelegramSyncResult | null>(null);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      setApiId("");
      setApiHash("");
      setPhone("");
      setCode("");
      setPassword("");
      setLoginState(null);
      setChannels([]);
      setSyncResult(null);
      setError(null);
      return;
    }
    let disposed = false;
    void invoke<TelegramConnectionStatus>("telegram_connection_status")
      .then((nextStatus) => { if (!disposed) setStatus(nextStatus); })
      .catch((reason) => { if (!disposed) setError(String(reason)); });
    return () => { disposed = true; };
  }, [open]);

  const run = async (action: string, operation: () => Promise<void>) => {
    if (busyAction) return;
    setBusyAction(action);
    setError(null);
    try {
      await operation();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusyAction(null);
    }
  };

  const saveCredentials = (event: FormEvent) => {
    event.preventDefault();
    void run("credentials", async () => {
      const nextStatus = await invoke<TelegramConnectionStatus>("telegram_save_credentials", {
        request: { apiId, apiHash },
      });
      setStatus(nextStatus);
      setApiId("");
      setApiHash("");
    });
  };

  const requestCode = (event: FormEvent) => {
    event.preventDefault();
    void run("phone", async () => {
      const nextState = await invoke<TelegramLoginState>("telegram_login_start", { request: { phone } });
      setLoginState(nextState);
      setPhone("");
      if (nextState.stage === "authorized") {
        setStatus(await invoke<TelegramConnectionStatus>("telegram_connection_status"));
      }
    });
  };

  const submitCode = (event: FormEvent) => {
    event.preventDefault();
    void run("code", async () => {
      const nextState = await invoke<TelegramLoginState>("telegram_login_code", { request: { code } });
      setLoginState(nextState);
      setCode("");
      if (nextState.stage === "authorized") {
        setStatus(await invoke<TelegramConnectionStatus>("telegram_connection_status"));
      }
    });
  };

  const submitPassword = (event: FormEvent) => {
    event.preventDefault();
    void run("password", async () => {
      const nextState = await invoke<TelegramLoginState>("telegram_login_password", { request: { password } });
      setLoginState(nextState);
      setPassword("");
      setStatus(await invoke<TelegramConnectionStatus>("telegram_connection_status"));
    });
  };

  const loadChannels = () => {
    void run("channels", async () => {
      setChannels(await invoke<TelegramChannel[]>("telegram_channels"));
    });
  };

  const saveChannels = () => {
    void run("selection", async () => {
      const nextChannels = await invoke<TelegramChannel[]>("telegram_select_channels", {
        request: { peerIds: channels.filter((channel) => channel.selected).map((channel) => channel.peerId) },
      });
      setChannels(nextChannels);
      setStatus(await invoke<TelegramConnectionStatus>("telegram_connection_status"));
    });
  };

  const syncChannels = () => {
    void run("sync", async () => {
      const result = await invoke<TelegramSyncResult>("telegram_sync_selected");
      setSyncResult(result);
    });
  };

  const deleteConnection = () => {
    if (!window.confirm("이 PC의 Telegram API 자격정보와 로그인 세션을 삭제할까요? 저장된 뉴스 리비전은 분석 재현을 위해 유지됩니다.")) return;
    void run("delete", async () => {
      setStatus(await invoke<TelegramConnectionStatus>("telegram_delete_connection"));
      setChannels([]);
      setSyncResult(null);
      setLoginState(null);
    });
  };

  return <section className="settings-provider-connect telegram-connect" aria-labelledby="telegram-connect-title">
    <header><div><span>TELEGRAM · SELECTED CHANNELS · READ ONLY</span><h3 id="telegram-connect-title">투자 뉴스 채널 수집</h3></div><b>{status.authorized ? "세션 저장됨" : status.configured ? "인증 필요" : "미연결"}</b></header>
    <p>{status.message} 방송 채널만 목록에 표시하며 메시지 전송·삭제·채널 가입·미디어 다운로드 기능은 포함하지 않습니다.</p>

    {!status.configured && <form onSubmit={saveCredentials}>
      <label htmlFor="telegram-api-id">Telegram API ID</label>
      <input id="telegram-api-id" inputMode="numeric" value={apiId} onChange={(event) => setApiId(event.currentTarget.value)} autoComplete="off" spellCheck={false} disabled={Boolean(busyAction)} />
      <label htmlFor="telegram-api-hash">Telegram API Hash</label>
      <input id="telegram-api-hash" type="password" value={apiHash} onChange={(event) => setApiHash(event.currentTarget.value)} autoComplete="new-password" spellCheck={false} disabled={Boolean(busyAction)} />
      <button className="settings-primary" type="submit" disabled={!apiId || !apiHash || Boolean(busyAction)}>{busyAction === "credentials" ? "저장 중…" : "API 자격정보 저장"}</button>
    </form>}

    {status.configured && !status.authorized && (!loginState || loginState.stage === "authorized") && <form onSubmit={requestCode}>
      <label htmlFor="telegram-phone">Telegram 전화번호</label>
      <input id="telegram-phone" type="tel" placeholder="+821012345678" value={phone} onChange={(event) => setPhone(event.currentTarget.value)} autoComplete="tel" disabled={Boolean(busyAction)} />
      <button className="settings-primary" type="submit" disabled={!phone || Boolean(busyAction)}>{busyAction === "phone" ? "인증 코드 요청 중…" : "인증 코드 요청"}</button>
    </form>}

    {loginState?.stage === "code" && <form onSubmit={submitCode}>
      <label htmlFor="telegram-code">Telegram 인증 코드</label>
      <input id="telegram-code" inputMode="numeric" value={code} onChange={(event) => setCode(event.currentTarget.value)} autoComplete="one-time-code" disabled={Boolean(busyAction)} />
      <button className="settings-primary" type="submit" disabled={!code || Boolean(busyAction)}>{busyAction === "code" ? "코드 확인 중…" : "인증 코드 확인"}</button>
    </form>}

    {loginState?.stage === "password" && <form onSubmit={submitPassword}>
      <label htmlFor="telegram-password">2단계 인증 비밀번호{loginState.passwordHint ? ` · 힌트: ${loginState.passwordHint}` : ""}</label>
      <input id="telegram-password" type="password" value={password} onChange={(event) => setPassword(event.currentTarget.value)} autoComplete="current-password" disabled={Boolean(busyAction)} />
      <button className="settings-primary" type="submit" disabled={!password || Boolean(busyAction)}>{busyAction === "password" ? "비밀번호 확인 중…" : "2단계 인증 확인"}</button>
    </form>}

    {loginState && <p className="telegram-login-message" role="status">{loginState.message}</p>}
    {error && <div className="settings-error" role="alert"><strong>텔레그램 작업을 완료하지 못했습니다.</strong><span>{error}</span></div>}

    {status.authorized && <div className="settings-provider-actions telegram-actions">
      <button className="settings-balance-button" type="button" onClick={loadChannels} disabled={Boolean(busyAction)}>{busyAction === "channels" ? "채널 불러오는 중…" : "방송 채널 불러오기"}</button>
      <button className="settings-balance-button" type="button" onClick={syncChannels} disabled={status.selectedChannelCount === 0 || Boolean(busyAction)}>{busyAction === "sync" ? "뉴스 동기화 중…" : "선택 채널 뉴스 동기화"}</button>
    </div>}

    {channels.length > 0 && <fieldset className="telegram-channel-list">
      <legend>수집 허용 채널 · {channels.filter((channel) => channel.selected).length}/{channels.length}</legend>
      {channels.map((channel) => <label key={channel.peerId}>
        <input type="checkbox" checked={channel.selected} onChange={() => setChannels((current) => current.map((item) => item.peerId === channel.peerId ? { ...item, selected: !item.selected } : item))} disabled={Boolean(busyAction)} />
        <span><strong>{channel.title}</strong><small>{channel.username ? `@${channel.username}` : "비공개 방송 채널"}</small></span>
      </label>)}
      <button className="settings-primary" type="button" onClick={saveChannels} disabled={Boolean(busyAction)}>{busyAction === "selection" ? "선택 저장 중…" : "수집 채널 선택 저장"}</button>
    </fieldset>}

    {syncResult && <div className="telegram-sync-result" role="status">
      <strong>동기화 결과</strong><span>{syncResult.message}</span>
      <ul>{syncResult.channels.map((channel) => <li key={channel.peerId} data-status={channel.status}>
        <b>{channel.title}</b><span>{channel.status === "synced" ? `${channel.fetchedMessageCount}건 확인 · ${channel.insertedRevisionCount}건 저장` : channel.error ?? "동기화 실패"}</span>
      </li>)}</ul>
      <small>{new Date(syncResult.syncedAtMs).toLocaleString("ko-KR")} · 원문 리비전 보존 · 중복 저장 방지</small>
    </div>}
    <small>API Hash·로그인 세션은 Windows 자격 증명 관리자에만 저장됩니다. 선택한 채널의 텍스트와 게시·수정·수집 시각만 로컬 DB에 보존하며 Codex 분석에는 사용자가 요청한 시점 범위만 전달합니다.</small>
    {status.configured && <div className="settings-provider-actions"><button className="settings-danger" type="button" onClick={deleteConnection} disabled={Boolean(busyAction)}>텔레그램 연결 삭제</button></div>}
  </section>;
}
