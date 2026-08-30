import { invoke, isTauri } from "@tauri-apps/api/core";
import { FormEvent, useCallback, useEffect, useState } from "react";

export type SocialAuthStatus = {
  googleConfigured: boolean;
  googleSecretConfigured: boolean;
  appleConfigured: boolean;
  googleMessage: string;
  appleMessage: string;
};

type WorkspaceIdentityStatus = {
  initialized: boolean;
  sessionAuthenticated: boolean;
  primaryProvider?: string | null;
  linkedProviders: string[];
  linkedAccountCount: number;
};

const EMPTY_STATUS: SocialAuthStatus = {
  googleConfigured: false,
  googleSecretConfigured: false,
  appleConfigured: false,
  googleMessage: "Google 로그인 설정을 확인하고 있습니다.",
  appleMessage: "Apple 로그인 설정을 확인하고 있습니다.",
};

const EMPTY_IDENTITY_STATUS: WorkspaceIdentityStatus = {
  initialized: false,
  sessionAuthenticated: false,
  primaryProvider: null,
  linkedProviders: [],
  linkedAccountCount: 0,
};

const providerLabel = (provider?: string | null) => ({ github: "GitHub", google: "Google", apple: "Apple" })[provider ?? ""] ?? "미설정";

export function SocialLoginSettings({ open }: { open: boolean }) {
  const [status, setStatus] = useState(EMPTY_STATUS);
  const [identityStatus, setIdentityStatus] = useState(EMPTY_IDENTITY_STATUS);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!isTauri()) return;
    try {
      const [social, identity] = await Promise.all([
        invoke<SocialAuthStatus>("social_auth_status"),
        invoke<WorkspaceIdentityStatus>("workspace_identity_status"),
      ]);
      setStatus(social);
      setIdentityStatus(identity);
    } catch (reason) {
      setMessage(String(reason));
    }
  }, []);

  useEffect(() => {
    if (open) void refresh();
  }, [open, refresh]);

  const saveGoogle = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setBusy(true);
    setMessage(null);
    try {
      setStatus(await invoke<SocialAuthStatus>("social_auth_save_google_client", { request: { clientId, clientSecret } }));
      setClientId("");
      setClientSecret("");
      setMessage("Google 데스크톱 OAuth Client ID와 Secret을 이 PC의 보안 저장소에 저장했습니다.");
    } catch (reason) {
      setMessage(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const deleteGoogle = async () => {
    setBusy(true);
    setMessage(null);
    try {
      setStatus(await invoke<SocialAuthStatus>("social_auth_delete_google_client"));
      setMessage("Google 로그인 설정을 이 PC에서 삭제했습니다.");
    } catch (reason) {
      setMessage(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const linkAccount = async (provider: "github" | "google") => {
    setBusy(true);
    setMessage(null);
    try {
      if (provider === "github") {
        await invoke("github_link_current_session");
      } else {
        await invoke("google_link_account");
      }
      await refresh();
      setMessage(`${providerLabel(provider)} 계정을 현재 Investa 작업공간에 연결했습니다.`);
    } catch (reason) {
      setMessage(String(reason));
    } finally {
      setBusy(false);
    }
  };

  return <div className="social-login-settings">
    <section className="workspace-identity-settings">
      <header><div><span>WORKSPACE OWNER</span><strong>작업공간 소유자와 연결 계정</strong></div><b>{identityStatus.initialized ? "소유자 고정" : "첫 로그인 대기"}</b></header>
      <p>{identityStatus.initialized
        ? `주 소유자: ${providerLabel(identityStatus.primaryProvider)} · 연결 계정 ${identityStatus.linkedAccountCount}개`
        : "처음 검증된 로그인 계정이 이 로컬 작업공간의 소유자가 됩니다."}</p>
      {identityStatus.linkedProviders.length > 0 && <ul aria-label="연결된 로그인 공급자">
        {identityStatus.linkedProviders.map((provider) => <li key={provider}>✓ {providerLabel(provider)} 연결됨</li>)}
      </ul>}
      <div className="workspace-link-actions">
        <button type="button" onClick={() => void linkAccount("github")} disabled={busy || !identityStatus.sessionAuthenticated}>현재 GitHub CLI 계정 연결</button>
        <button type="button" onClick={() => void linkAccount("google")} disabled={busy || !identityStatus.sessionAuthenticated || !status.googleConfigured}>Google 계정 연결</button>
      </div>
      <small>{identityStatus.sessionAuthenticated ? "현재 소유자 세션에서 새 계정을 연결할 수 있습니다." : "계정 연결은 소유자로 로그인한 세션에서만 가능합니다."} 연결하지 않은 계정은 같은 Windows 사용자여도 이 작업공간을 열 수 없습니다.</small>
    </section>
    <details className="social-login-advanced">
      <summary>개발자용 OAuth 설정</summary>
      <div>
        <section>
          <header><div><span>GOOGLE · DESKTOP OAUTH</span><strong>Google OAuth 앱 설정</strong></div><b>{status.googleConfigured ? "준비됨" : "미설정"}</b></header>
          <p>{status.googleMessage}</p>
          <form onSubmit={(event) => void saveGoogle(event)}>
            <label htmlFor="google-oauth-client-id">데스크톱 앱 OAuth Client ID</label>
            <input
              id="google-oauth-client-id"
              value={clientId}
              onChange={(event) => setClientId(event.currentTarget.value)}
              placeholder="…apps.googleusercontent.com"
              autoComplete="off"
              spellCheck={false}
              disabled={busy}
            />
            <label htmlFor="google-oauth-client-secret">데스크톱 앱 OAuth Client Secret</label>
            <input
              id="google-oauth-client-secret"
              type="password"
              value={clientSecret}
              onChange={(event) => setClientSecret(event.currentTarget.value)}
              placeholder={status.googleSecretConfigured ? "저장됨 · 변경할 때만 다시 입력" : "Google Cloud에서 새로 발급"}
              autoComplete="new-password"
              spellCheck={false}
              disabled={busy}
            />
            <div><button className="settings-primary" type="submit" disabled={busy || !clientId.trim() || !clientSecret.trim()}>보안 저장소에 저장</button>{status.googleConfigured && <button type="button" onClick={() => void deleteGoogle()} disabled={busy}>설정 삭제</button>}</div>
          </form>
          <small>일반 사용자는 위의 `Google 계정 연결`만 사용합니다. 이 설정은 OAuth 앱 교체·복구용이며 Client Secret은 Windows 자격 증명 관리자에만 저장합니다.</small>
        </section>
        <section className="is-deferred">
          <header><div><span>APPLE · WEB AUTHORIZATION</span><strong>Apple ID 로그인</strong></div><b>{status.appleConfigured ? "준비됨" : "Developer 설정 필요"}</b></header>
          <p>{status.appleMessage}</p>
          <small>Apple은 Services ID, 검증된 도메인, HTTPS 콜백과 서버측 토큰 검증을 갖춘 뒤 활성화합니다.</small>
        </section>
      </div>
    </details>
    {message && <p className="settings-info" role="status">{message}</p>}
  </div>;
}
