import { type FormEvent, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export type CodexConnectionStatus = {
  available: boolean;
  connected: boolean;
  loggedIn: boolean;
  version?: string | null;
  authMode?: string | null;
  message: string;
};

type AiProviderId = "claude" | "antigravity";

type AiProviderStatus = {
  provider: AiProviderId;
  label: string;
  configured: boolean;
  connected: boolean;
  model: string;
  paidApi: boolean;
  analysisOnly: boolean;
  message: string;
};

const PROVIDER_COPY: Record<AiProviderId, { keyLabel: string; modelLabel: string; modelEditable: boolean; note: string }> = {
  claude: {
    keyLabel: "Anthropic API Key",
    modelLabel: "Claude model",
    modelEditable: true,
    note: "Claude 구독과 API 과금은 별도입니다. 저장만으로 호출하거나 비용을 발생시키지 않습니다.",
  },
  antigravity: {
    keyLabel: "Gemini API Key",
    modelLabel: "Managed agent",
    modelEditable: false,
    note: "Google AI Pro 구독과 Gemini API 과금·쿼터는 별도입니다. 검색·URL 읽기만 허용하고 코드 실행·파일·주문 도구는 제공하지 않습니다.",
  },
};

const isTauriRuntime = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function AiProviderConnections({ open, codexStatus }: { open: boolean; codexStatus: CodexConnectionStatus | null }) {
  const [statuses, setStatuses] = useState<AiProviderStatus[]>([]);
  const [keys, setKeys] = useState<Record<AiProviderId, string>>({ claude: "", antigravity: "" });
  const [models, setModels] = useState<Record<AiProviderId, string>>({ claude: "claude-sonnet-4-6", antigravity: "antigravity-preview-05-2026" });
  const [busy, setBusy] = useState<AiProviderId | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open || !isTauriRuntime) return;
    let disposed = false;
    void invoke<AiProviderStatus[]>("ai_provider_statuses")
      .then((next) => {
        if (disposed) return;
        setStatuses(next);
        setModels((current) => next.reduce((result, item) => ({ ...result, [item.provider]: item.model }), current));
      })
      .catch((reason) => { if (!disposed) setError(String(reason)); });
    return () => { disposed = true; };
  }, [open]);

  const save = async (event: FormEvent, provider: AiProviderId) => {
    event.preventDefault();
    if (!keys[provider] || busy) return;
    setBusy(provider);
    setError(null);
    try {
      const next = await invoke<AiProviderStatus>("ai_provider_save_config", {
        request: { provider, apiKey: keys[provider], model: models[provider] },
      });
      setStatuses((current) => [...current.filter((item) => item.provider !== provider), next]);
      setKeys((current) => ({ ...current, [provider]: "" }));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  };

  const remove = async (provider: AiProviderId) => {
    if (busy || !window.confirm(`${PROVIDER_COPY[provider].keyLabel}를 이 PC에서 삭제할까요?`)) return;
    setBusy(provider);
    setError(null);
    try {
      const next = await invoke<AiProviderStatus>("ai_provider_delete_config", { provider });
      setStatuses((current) => [...current.filter((item) => item.provider !== provider), next]);
      setKeys((current) => ({ ...current, [provider]: "" }));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  };

  return <section className="settings-ai-connections" aria-labelledby="ai-connections-title">
    <div><span>AI PROVIDERS</span><h3 id="ai-connections-title">분석 모델 연결</h3></div>
    <article className={codexStatus?.connected ? "is-connected" : ""}>
      <i aria-hidden="true" />
      <div><strong>Codex</strong><span>{codexStatus?.connected ? `${codexStatus.version ?? "CLI"} · ${codexStatus.authMode ?? "로그인"}` : codexStatus?.message ?? "상태 확인 중"}</span></div>
      <b>{codexStatus?.connected ? "연결됨" : "사용자 설정 필요"}</b>
    </article>
    {(["claude", "antigravity"] as const).map((provider) => {
      const status = statuses.find((item) => item.provider === provider);
      const copy = PROVIDER_COPY[provider];
      return <div className="settings-ai-provider" key={provider}>
        <article className={status?.configured ? "is-connected" : ""}>
          <i aria-hidden="true" />
          <div><strong>{status?.label ?? (provider === "claude" ? "Claude API" : "Google Antigravity")}</strong><span>{status?.message ?? "어댑터 상태 확인 중"}</span></div>
          <b>{status?.configured ? "키 저장됨" : "미연결"}</b>
        </article>
        <form onSubmit={(event) => void save(event, provider)}>
          <label htmlFor={`${provider}-api-key`}>{copy.keyLabel}</label>
          <input id={`${provider}-api-key`} type="password" value={keys[provider]} onChange={(event) => setKeys((current) => ({ ...current, [provider]: event.currentTarget.value }))} autoComplete="new-password" spellCheck={false} disabled={busy === provider} />
          <label htmlFor={`${provider}-model`}>{copy.modelLabel}</label>
          <input id={`${provider}-model`} value={models[provider]} onChange={(event) => setModels((current) => ({ ...current, [provider]: event.currentTarget.value }))} readOnly={!copy.modelEditable} spellCheck={false} disabled={busy === provider} />
          <button className="settings-primary" type="submit" disabled={!keys[provider] || busy !== null}>{busy === provider ? "저장 중…" : "이 PC에 안전하게 저장"}</button>
        </form>
        <p>{copy.note}</p>
        {status?.configured && <button className="settings-danger" type="button" onClick={() => void remove(provider)} disabled={busy !== null}>연결 정보 삭제</button>}
      </div>;
    })}
    {error && <div className="settings-error" role="alert"><strong>AI 공급자 설정을 처리하지 못했습니다.</strong><span>{error}</span></div>}
    <p>추가 공급자는 같은 분석 전용 계약을 상속합니다. AI에는 계좌 키·잔고·주문 함수·위험 정책 변경 권한을 제공하지 않습니다.</p>
  </section>;
}
