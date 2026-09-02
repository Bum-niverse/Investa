import { invoke, isTauri } from "@tauri-apps/api/core";
import { FormEvent, useCallback, useEffect, useState } from "react";

type Status = {
  opendartConfigured: boolean;
  naverNewsConfigured: boolean;
  readOnly: true;
  message: string;
};

const EMPTY: Status = {
  opendartConfigured: false,
  naverNewsConfigured: false,
  readOnly: true,
  message: "공식 국내 공시·뉴스 설정을 확인하고 있습니다.",
};

export function OfficialKrDataSettings({ open }: { open: boolean }) {
  const [status, setStatus] = useState(EMPTY);
  const [dartKey, setDartKey] = useState("");
  const [naverId, setNaverId] = useState("");
  const [naverSecret, setNaverSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!isTauri()) return;
    setStatus(await invoke<Status>("official_kr_data_status"));
  }, []);

  useEffect(() => { if (open) void refresh().catch((reason) => setMessage(String(reason))); }, [open, refresh]);

  const save = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setBusy(true);
    setMessage(null);
    try {
      const request = {
        opendartApiKey: dartKey.trim() || undefined,
        naverClientId: naverId.trim() || undefined,
        naverClientSecret: naverSecret.trim() || undefined,
      };
      setStatus(await invoke<Status>("official_kr_data_save_config", { request }));
      setDartKey(""); setNaverId(""); setNaverSecret("");
      setMessage("입력한 공식 데이터 자격정보를 이 PC의 보안 저장소에 저장했습니다. 실제 조회 전까지 연결 완료로 표시하지 않습니다.");
    } catch (reason) { setMessage(String(reason)); }
    finally { setBusy(false); }
  };

  const remove = async (target: "dart" | "naver") => {
    if (!window.confirm(`${target === "dart" ? "OpenDART" : "네이버 뉴스"} 자격정보를 이 PC에서 삭제할까요?`)) return;
    setBusy(true); setMessage(null);
    try {
      const request = target === "dart"
        ? { opendartApiKey: "" }
        : { naverClientId: "", naverClientSecret: "" };
      setStatus(await invoke<Status>("official_kr_data_save_config", { request }));
      setMessage("선택한 공식 데이터 자격정보를 삭제했습니다.");
    } catch (reason) { setMessage(String(reason)); }
    finally { setBusy(false); }
  };

  return <section className="settings-provider-connect" aria-labelledby="official-kr-data-title">
    <header><div><span>KR OFFICIAL DATA · READ ONLY</span><h3 id="official-kr-data-title">OpenDART·네이버 뉴스</h3></div><b>{status.opendartConfigured || status.naverNewsConfigured ? "설정 저장됨" : "미설정"}</b></header>
    <p>{status.message} OpenDART 회사 고유번호는 KRX 종목코드와 다르며, 뉴스 본문은 신뢰할 수 없는 외부 근거로 취급합니다.</p>
    <form onSubmit={(event) => void save(event)}>
      <label htmlFor="opendart-api-key">OpenDART API 키</label><input id="opendart-api-key" type="password" value={dartKey} onChange={(event) => setDartKey(event.currentTarget.value)} placeholder={status.opendartConfigured ? "저장됨 · 변경할 때만 입력" : "금융감독원에서 발급"} autoComplete="new-password" disabled={busy} />
      <label htmlFor="naver-client-id">네이버 Client ID</label><input id="naver-client-id" value={naverId} onChange={(event) => setNaverId(event.currentTarget.value)} placeholder={status.naverNewsConfigured ? "저장됨 · 변경할 때 Secret과 함께 입력" : "개발자 센터에서 발급"} autoComplete="off" disabled={busy} />
      <label htmlFor="naver-client-secret">네이버 Client Secret</label><input id="naver-client-secret" type="password" value={naverSecret} onChange={(event) => setNaverSecret(event.currentTarget.value)} placeholder={status.naverNewsConfigured ? "저장됨 · 변경할 때 ID와 함께 입력" : "개발자 센터에서 발급"} autoComplete="new-password" disabled={busy} />
      <button className="settings-primary" type="submit" disabled={busy || (!dartKey.trim() && !(naverId.trim() && naverSecret.trim()))}>{busy ? "저장 중…" : "보안 저장소에 저장"}</button>
    </form>
    <div className="settings-provider-actions">{status.opendartConfigured && <button className="settings-danger" type="button" disabled={busy} onClick={() => void remove("dart")}>OpenDART 키 삭제</button>}{status.naverNewsConfigured && <button className="settings-danger" type="button" disabled={busy} onClick={() => void remove("naver")}>네이버 키 삭제</button>}</div>
    {message && <p className="settings-info" role="status">{message}</p>}
  </section>;
}
