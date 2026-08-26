import { invoke, isTauri } from "@tauri-apps/api/core";
import { useState, type ReactNode } from "react";

type GithubUser = { id: number; login: string; name?: string | null; avatarUrl: string };

function TradingChartPreview() {
  const workers = ["tablet", "talk", "laptop", "coffee", "documents", "phone", "tablet"];
  const useReferenceArtwork = true;
  if (useReferenceArtwork) return <div className="login-image-hero">
    <img src="/login-full-lounge.png" alt="부서별 넥타이를 착용한 직원들이 일하고 쉬는 Investa 투자본부 카페 라운지" />
    <div className="image-scene-motion" aria-hidden="true">
      <i className="image-motion screen-left" /><i className="image-motion tablet-center" /><i className="image-motion laptop-center" />
      <i className="image-motion coffee-bar" /><i className="image-motion phone-right" />
      <i className="image-motion dialogue-a">···</i><i className="image-motion dialogue-b">··</i>
    </div>
  </div>;
  return <div className="login-css-hero">
    <div className="css-hero-copy"><span>LOCAL-FIRST AI TRADING LAB</span><h1>당신만의 투자본부를<br />설립하세요</h1><p>분석 백테스트 모의투자 기록은 당신의 계정에만 저장됩니다.<br />분석부터 자동 매매까지, 한 곳에서.</p></div>
    <section className="css-market-board" aria-label="시뮬레이션 시장 차트">
      <header><b>INVESTA MARKET LAB</b><span>POINT-IN-TIME · SHADOW DATA</span></header>
      <div className="css-market-summary"><span>KOSPI</span><strong>2,742.31</strong><i>+1.24%</i><small>1D · MA20 · VOLUME</small></div>
      <svg viewBox="0 0 760 100" preserveAspectRatio="none" aria-hidden="true"><polyline points="0,78 45,71 90,73 135,57 180,61 225,45 270,53 315,36 360,43 405,29 450,35 495,22 540,29 585,17 630,22 680,10 730,15 760,7" /><polyline className="css-signal" points="0,86 65,78 130,71 195,62 260,55 325,47 390,40 455,34 520,27 585,22 650,17 715,12 760,10" /></svg>
      <div className="css-candles">{Array.from({ length: 18 }, (_, index) => <i className={index % 5 === 1 || index % 7 === 0 ? "down" : "up"} key={index} />)}</div>
    </section>
    <section className="css-lounge" aria-label="직원들이 일하고 대화하는 투자본부 카페 라운지">
      <div className="css-windows"><i /><i /><i /></div><div className="css-wall-art" /><div className="css-shelf"><i /><i /><i /><i /></div>
      <div className="css-cafe-bar"><span>INVESTA CAFE</span><i /><i /><i /></div><div className="css-counter-lamp lamp-a" /><div className="css-counter-lamp lamp-b" />
      <div className="css-sofa sofa-left" /><div className="css-sofa sofa-center" /><div className="css-sofa sofa-right" />
      <div className="css-coffee-table table-front"><i /><b /></div><div className="css-coffee-table table-side"><i /></div><div className="css-high-table"><i /><i /></div>
      <div className="css-plant plant-a"><i /><i /><i /><i /></div><div className="css-plant plant-b"><i /><i /><i /></div><div className="css-plant plant-c"><i /><i /><i /></div>
      {workers.map((activity, index) => <span className={`css-worker worker-${index + 1} dept-${index + 1} activity-${activity}`} key={`${activity}-${index}`}>
        <i className="css-worker-hair" /><i className="css-worker-face"><b /><b /></i><i className="css-worker-body"><b /></i><i className="css-worker-legs" />
        {activity === "coffee" && <i className="css-prop-coffee" />}{activity === "documents" && <i className="css-prop-documents" />}{activity === "tablet" && <i className="css-prop-tablet" />}{activity === "laptop" && <i className="css-prop-laptop" />}{activity === "phone" && <i className="css-prop-phone" />}
      </span>)}
    </section>
  </div>;
  /* Legacy code retained temporarily for a narrow visual rollback.
  const candles = [46, 58, 39, 68, 52, 77, 61, 83, 70, 92, 76, 86, 66, 80, 72, 96, 84, 102];
  const volumes = [24, 42, 31, 58, 37, 66, 48, 78, 51, 88, 64, 73, 45, 69, 56, 91, 68, 82];
  return <div className="login-chart-preview" aria-hidden="true">
    <header><strong>INVESTA MARKET LAB</strong><span>POINT-IN-TIME · SHADOW DATA</span></header>
    <div className="login-chart-toolbar"><span>KOSPI</span><b>2,742.31</b><i>+1.24%</i><small>1D · MA20 · VOLUME</small></div>
    <div className="login-chart-canvas">
      <svg className="login-chart-lines" viewBox="0 0 720 250" preserveAspectRatio="none">
        <polyline className="line-primary" points="0,190 38,178 76,184 114,151 152,160 190,124 228,139 266,101 304,116 342,82 380,96 418,65 456,78 494,54 532,69 570,38 608,48 646,25 684,37 720,18" />
        <polyline className="line-signal" points="0,205 38,194 76,181 114,176 152,160 190,151 228,133 266,128 304,112 342,104 380,91 418,84 456,73 494,68 532,58 570,52 608,43 646,38 684,31 720,27" />
      </svg>
      <div className="login-chart-candles">{candles.map((height, index) => <i className={index % 4 === 1 || index % 7 === 0 ? "is-down" : "is-up"} key={index} style={{ "--login-candle-height": `${height}px`, "--login-candle-shift": `${(index % 5) * 7}px` } as CSSProperties} />)}</div>
      <div className="login-chart-volume">{volumes.map((height, index) => <i className={index % 4 === 1 || index % 7 === 0 ? "is-down" : "is-up"} key={index} style={{ height: `${height}%` }} />)}</div>
      <span className="login-chart-price price-a">2,700</span><span className="login-chart-price price-b">2,600</span><span className="login-chart-price price-c">2,500</span>
    </div>
    <footer><span><i className="legend-price" />PRICE</span><span><i className="legend-ma" />MA 20</span><strong>SIMULATION VIEW · NOT A LIVE QUOTE</strong></footer>
    <div className="login-trader-floor">
      <span className="lounge-window"><i /><i /><i /></span>
      <span className="lounge-sofa"><i /><i /><i /></span>
      <span className="lounge-table table-left"><i /><b /></span>
      <span className="lounge-table table-right"><i /><b /></span>
      <span className="lounge-counter"><i /><i /><b>INVESTA CAFE</b></span>
      <span className="lounge-plant plant-left"><i /><i /><i /></span>
      <span className="lounge-plant plant-right"><i /><i /><i /></span>
      {[
        { activity: "documents", pose: "seated" },
        { activity: "coffee", pose: "standing" },
        { activity: "briefing", pose: "standing" },
        { activity: "tablet", pose: "seated" },
        { activity: "coffee", pose: "seated" },
      ].map(({ activity, pose }, index) => <span className={`login-pixel-trader trader-${index + 1} is-${pose} has-${activity}`} key={`${activity}-${index}`}>
        <i className="trader-hair" /><i className="trader-face"><b /><b /></i><i className="trader-suit"><b /></i><i className="trader-legs" />
        {activity === "coffee" && <i className="trader-coffee" />}
        {activity === "documents" && <i className="trader-documents" />}
        {activity === "briefing" && <i className="trader-briefing">···</i>}
        {activity === "tablet" && <i className="trader-tablet" />}
      </span>)}
    </div>
  </div>; */
}

export function GitHubLoginGate({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<GithubUser | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const verifySession = async () => {
    if (busy) return;
    setBusy(true);
    setMessage(null);
    try {
      if (!isTauri()) throw new Error("GitHub 로그인은 Investa 데스크톱 앱에서 확인할 수 있습니다.");
      setUser(await invoke<GithubUser>("github_session"));
    } catch (reason) {
      setMessage(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const startLogin = async () => {
    setMessage(null);
    try {
      if (!isTauri()) throw new Error("GitHub CLI 로그인은 Investa 데스크톱 앱에서 시작할 수 있습니다.");
      await invoke("github_login_start");
      setMessage("열린 GitHub CLI 창에서 로그인을 마친 뒤 ‘GitHub 세션 확인’을 눌러 주세요.");
    } catch (reason) {
      setMessage(String(reason));
    }
  };

  if (user) return <>{children}</>;

  return <main className="investa-login-shell">
    <section className="investa-login-visual" aria-label="AI 투자본부 픽셀 카페 라운지">
      <TradingChartPreview />
      <div className="login-visual-copy"><span>LOCAL-FIRST AI TRADING LAB</span><h1>당신만의 투자본부를<br />설립하세요</h1><p>분석 백테스트 모의투자 기록은 당신의 계정에만 저장됩니다.<br />분석부터 자동 매매까지, 한 곳에서.</p></div>
    </section>
    <section className="investa-login-panel">
      <div className="login-brand-mark" aria-hidden="true">IV</div>
      <p className="login-eyebrow">SECURE LOCAL WORKSPACE</p>
      <h2>GitHub 계정으로 시작</h2>
      <p className="login-description">GitHub CLI의 기존 로그인 세션으로 사용자만 확인합니다. Investa는 GitHub 토큰을 저장하지 않으며 거래소·증권사 자격정보와 로그인 정보를 분리합니다.</p>
      <div className="login-boundaries">
        <span><i aria-hidden="true" />GitHub 토큰 저장 안 함</span>
        <span><i aria-hidden="true" />금융 API 키는 Windows 보안 저장소 사용</span>
        <span><i aria-hidden="true" />로그인 후에도 실전 주문은 잠금</span>
      </div>
      {message && <p className="login-feedback" role="status">{message}</p>}
      <button className="login-github-primary" disabled={busy} onClick={() => void verifySession()} type="button"><span aria-hidden="true">GH</span>{busy ? "GitHub 계정 확인 중…" : "GitHub 세션 확인"}</button>
      <button className="login-github-secondary" onClick={() => void startLogin()} type="button">GitHub CLI 로그인 시작</button>
      <div className="login-divider"><span>향후 공급자</span></div>
      <div className="login-provider-slots" aria-label="향후 지원 검토 중인 로그인 공급자">
        <button disabled type="button">Google <small>검토 중</small></button>
        <button disabled type="button">Apple <small>검토 중</small></button>
      </div>
      <small className="login-local-note">이 로그인은 앱 진입을 구분하지만 로컬 SQLite 자체를 암호화하지는 않습니다. Windows 계정 잠금도 함께 사용하세요.</small>
    </section>
  </main>;
}
