import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { forecastAssetMarket, type AnalysisMarket } from "./analysisMarket";
import { TechnicalChartEvidenceView } from "./TechnicalChartEvidenceView";
import type { TechnicalChartEvidence } from "./technicalChartEvidence";
import { PortfolioOverview, type PortfolioRecordSnapshot } from "./PortfolioOverview";
import type { ExternalEvidenceSource } from "./externalEvidenceSources";

type AnalysisFilter = "all" | "strategy" | Exclude<AnalysisMarket, "mixed">;
type AnalysisClassification = "all" | "system_check" | "research_experiment" | "promotion_candidate";
type DepartmentRecordReport = {
  departmentName: string;
  conclusion: "proceed" | "watch" | "reject" | "out_of_scope";
  confidencePercent: number;
  summary: string;
  roleFindings: Array<{ agentId: string; role: string; finding: string; evidenceIds: string[]; counterevidence: string[]; evidenceGap?: string | null }>;
  risks: string[];
  nextActions: string[];
};
type AnalysisSummary = {
  recordId: string;
  kind: "strategy" | "instrument" | "meeting";
  status: "completed" | "blocked" | "held" | "error";
  market: AnalysisMarket;
  title: string;
  symbol: string;
  currency: string;
  requestedAtMs?: number | null;
  completedAtMs: number;
  priceLowMinor?: number | null;
  priceHighMinor?: number | null;
  totalReturnBps?: number | null;
  maxDrawdownBps?: number | null;
  winRateBps?: number | null;
  completedTradeCount?: number | null;
  classification: Exclude<AnalysisClassification, "all">;
};
type AnalysisDetail = {
  summary: AnalysisSummary;
  record: {
    type?: "meeting" | "strategy_review" | "role_report" | "department_delegation";
    topic?: string;
    synthesis?: { decision: string; summary: string; consensus: string[]; disagreements: string[]; conditions: string[] };
    reports?: Record<string, DepartmentRecordReport>;
    portfolio?: PortfolioRecordSnapshot;
    portfolioCharts?: TechnicalChartEvidence[];
    telegramEvidence?: {
      provider: string;
      asOfMs?: number | null;
      totalAvailableCount: number;
      includedCount: number;
      snapshotCandidateCount?: number;
      retrievedCount?: number;
      selectedSourceCount: number;
      syncStatus: string;
      message: string;
    };
    evidenceSources?: ExternalEvidenceSource[];
    review?: { executable: boolean; issues: Array<{ field: string; message: string }> };
    departmentReport?: DepartmentRecordReport;
    report?: {
      request?: string;
      evidence?: Array<{ evidenceId: string; kind?: string; sourceUrl?: string; summary?: string; source?: string; sourceRevision?: string | null; observation?: string; counterevidence?: string[]; observedAt?: string | null }>;
      strategyCandidate?: { hypothesis: string; limitations: string[]; unknowns: string[] };
      agentId?: string;
      role?: string;
      scope?: string;
      stance?: "supportive" | "critical" | "neutral" | "not_applicable";
      confidencePercent?: number;
      summary?: string;
      findings?: string[];
      evidenceGaps?: string[];
      nextRequests?: string[];
    };
    chartEvidence?: TechnicalChartEvidence | null;
    provider?: string;
    interval?: string;
    adjusted?: boolean;
    warnings?: string[];
    config?: BacktestConfig;
    result?: {
      experimentId?: string;
      initialCashMinor?: number;
      finalEquityMinor?: number;
      realizedPnlMinor?: number;
      totalReturnBps?: number;
      maxDrawdownBps?: number;
      winRateBps?: number | null;
      completedTradeCount?: number;
      inputBarCount: number;
      profitFactorMilli?: number | null;
      performance?: {
        annualizedVolatilityBps?: number | null;
        sharpeRatioMilli?: number | null;
        sortinoRatioMilli?: number | null;
        priceBenchmarkReturnBps: number;
        alphaVsPriceBenchmarkBps: number;
      } | null;
      patternProbability?: {
        minimumPublishedSample: number;
        currentSequenceDirection: "bullish" | "bearish" | "doji";
        currentSequenceCount: number;
        nextCandle: {
          sampleSize: number;
          bullishProbabilityBps?: number | null;
          bearishProbabilityBps?: number | null;
          dojiProbabilityBps?: number | null;
          bullishConfidenceIntervalBps?: { low: number; high: number } | null;
        };
        horizonOutcomes: Array<{
          horizonBars: number;
          sampleSize: number;
          positiveProbabilityBps?: number | null;
          negativeProbabilityBps?: number | null;
        }>;
        bollinger: {
          currentPosition: string;
          upperSampleSize: number;
          upperBreakoutProbabilityBps?: number | null;
          upperReversalProbabilityBps?: number | null;
          lowerSampleSize: number;
          lowerBounceProbabilityBps?: number | null;
          lowerBreakdownProbabilityBps?: number | null;
        };
        warnings: string[];
      } | null;
    };
  };
};
type BacktestConfig = {
  experimentId: string;
  datasetId: string;
  initialCashMinor: number;
  orderQuantity: number;
  quantityScale: number;
  closeOpenPositionAtEnd: boolean;
  costs: { buyFeeBps: number; sellFeeBps: number; sellTaxBps: number; slippageBps: number };
  riskLimits?: { stopLossBps: number; takeProfitBps: number; dailyLossLimitMinor: number } | null;
};
type CloneDraft = {
  fastWindow: string; slowWindow: string; initialCashMinor: string; orderQuantity: string;
  buyFeeBps: string; sellFeeBps: string; sellTaxBps: string; slippageBps: string;
  stopLossBps: string; takeProfitBps: string; dailyLossLimitMinor: string;
  closeOpenPositionAtEnd: boolean;
};
type ExperimentComparison = {
  sourceExperimentId: string;
  clonedExperimentId: string;
  sourceConfig: BacktestConfig;
  clonedConfig: BacktestConfig;
  sourceResult: NonNullable<AnalysisDetail["record"]["result"]>;
  clonedResult: NonNullable<AnalysisDetail["record"]["result"]>;
};
type WalkForwardReport = {
  validationRunId: string;
  createdAtMs: number;
  sourceExperimentId: string;
  strategyTrialCount: number;
  foldCount: number;
  initialTrainingBarCount: number;
  positiveOosFoldCount: number;
  largestAbsoluteOosReturnShareBps: number;
  oosReturnSpreadBps: number;
  totalOosTradeCount: number;
  minimumOosTradeCount: number;
  meetsResearchSampleMinimum: boolean;
  promotionBlockers: string[];
  promotionEvaluation: { policyVersion: string; eligibleForPaperReview: boolean; checks: Array<{ checkId: string; label: string; passed: boolean; observed: string; required: string }>; warning: string };
  overfitDiagnostics: { comparableStrategyCount: number; evaluatedPartitionCount: number; probabilityOfBacktestOverfittingBps?: number | null; deflatedSharpeRatioMilli?: number | null; minimumTrackRecordLength?: number | null; blockers: string[] };
  folds: Array<{
    foldNumber: number;
    trainingBarCount: number;
    oosBarCount: number;
    trainingEndMs: number;
    oosStartMs: number;
    oosEndMs: number;
    training: { totalReturnBps: number; maxDrawdownBps: number; completedTradeCount: number; winRateBps?: number | null; profitFactorMilli?: number | null; expectedTradePnlMinor?: number | null; turnoverBps: number; exposureBps: number };
    outOfSample: { totalReturnBps: number; maxDrawdownBps: number; completedTradeCount: number; winRateBps?: number | null; profitFactorMilli?: number | null; expectedTradePnlMinor?: number | null; turnoverBps: number; exposureBps: number };
    regimes: Array<{ regime: "bullish" | "bearish" | "sideways" | "high_volatility"; completedTradeCount: number; winningTradeCount: number; realizedPnlMinor: number; winRateBps?: number | null }>;
    unclassifiedTradeCount: number;
  }>;
  warnings: string[];
};
type ForecastTrace = {
  forecast: {
    assetClass: "korea_stock" | "united_states_stock" | "equity_future" | "index_future" | "crypto_spot" | "crypto_perpetual";
    evidenceMode: "full_features" | "price_only_fallback" | "unavailable";
    createdAtMs: number;
    forecast: {
      forecastId: string; modelId: string; modelVersion: string; datasetId: string; assetContractId: string;
      horizonMs: number; generatedAtMs: number; upProbabilityBps?: number | null; downProbabilityBps?: number | null;
      flatProbabilityBps?: number | null; recommendationConfidenceBps?: number | null; modelReliabilityBps?: number | null;
      unavailableReason?: string | null; priceOnlyFallback: boolean;
    };
  };
  calibration?: {
    calibrationId: string; createdAtMs: number;
    metrics: { sampleCount: number; brierScoreMillionths: number; logLossMillionths: number; expectedCalibrationErrorBps: number; populatedBinCount: number };
  } | null;
};
type MarketScreenerPreset = "balanced" | "momentum" | "reversal";
type MarketScreenerCandidate = {
  symbol: string; name: string; market: string; currency: string; latestPriceMinor: number;
  coarseScoreBps: number; screeningScoreBps: number; twentyDayReturnBps: number;
  rsi14Bps: number; averageVolume20d: number; reasons: string[];
};
type MarketScreenerResult = {
  provider: string; market: "kr" | "us"; preset: MarketScreenerPreset; asOfMs: number;
  rankedAt: string[]; coarseUniverseCount: number; technicalEvaluatedCount: number;
  excludedCount: number; candidates: MarketScreenerCandidate[]; warnings: string[];
  errors: string[]; liveOrderEnabled: boolean;
};

const FILTERS: Array<{ id: AnalysisFilter; label: string }> = [
  { id: "all", label: "전체" },
  { id: "strategy", label: "전략" },
  { id: "kr", label: "국장" },
  { id: "us", label: "미장" },
  { id: "coin", label: "코인" },
  { id: "securities_futures", label: "증권 선물" },
  { id: "crypto_futures", label: "코인 선물" },
];
const MARKET_LABELS: Record<AnalysisMarket, string> = { kr: "국내주식", us: "미국주식", coin: "코인", securities_futures: "증권 선물", crypto_futures: "코인 선물", mixed: "복합 안건" };
const CLASSIFICATION_LABELS = { system_check: "시스템 검사", research_experiment: "연구 실험", promotion_candidate: "승격 후보" } as const;
const dateKey = (timestamp: number) => {
  const date = new Date(timestamp);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
};
const formatDateTime = (timestamp?: number | null) => timestamp
  ? new Date(timestamp).toLocaleString("ko-KR", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" })
  : "과거 기록 · 요청 시각 미기록";
const formatMoney = (minor: number, currency: string) => new Intl.NumberFormat("ko-KR", {
  style: "currency",
  currency,
  maximumFractionDigits: currency === "KRW" ? 0 : 2,
}).format(minor / (currency === "KRW" ? 1 : 100));
const formatPercent = (bps?: number | null) => bps == null ? "계산 불가" : `${(bps / 100).toFixed(2)}%`;
const formatRatio = (milli?: number | null) => milli == null ? "계산 불가" : (milli / 1_000).toFixed(3);
const directionLabel = (direction?: "bullish" | "bearish" | "doji") => ({ bullish: "양봉", bearish: "음봉", doji: "도지" })[direction ?? "doji"];
const statusLabel = (status: AnalysisSummary["status"]) => ({ completed: "완료", blocked: "차단", held: "보류", error: "오류" })[status];
const regimeLabel = (regime: WalkForwardReport["folds"][number]["regimes"][number]["regime"]) => ({ bullish: "상승 관측", bearish: "하락 관측", sideways: "횡보 관측", high_volatility: "고변동 관측" })[regime];
const forecastAssetLabel: Record<ForecastTrace["forecast"]["assetClass"], string> = { korea_stock: "국내주식", united_states_stock: "미국주식", equity_future: "주식선물", index_future: "지수선물", crypto_spot: "코인현물", crypto_perpetual: "코인선물" };
const evidenceModeLabel: Record<ForecastTrace["forecast"]["evidenceMode"], string> = { full_features: "전체 피처", price_only_fallback: "가격 전용 fallback", unavailable: "산출 불가" };

const conclusionLabel = (conclusion: DepartmentRecordReport["conclusion"]) => ({
  proceed: "검토 진행",
  watch: "관찰",
  reject: "기각",
  out_of_scope: "범위 밖",
})[conclusion];

function DepartmentReportPanel({ id, report }: { id: string; report: DepartmentRecordReport }) {
  const uniqueEvidenceIds = new Set(report.roleFindings.flatMap((item) => item.evidenceIds));
  const evidencedRoleCount = report.roleFindings.filter((item) => item.evidenceIds.length > 0).length;
  const gapRoleCount = report.roleFindings.filter((item) => Boolean(item.evidenceGap)).length;
  return <details className="department-report-panel" open>
    <summary>
      <span><small>DEPARTMENT REPORT</small><strong>{report.departmentName}</strong></span>
      <span className="department-report-score"><b>{report.confidencePercent}%</b><small>{conclusionLabel(report.conclusion)} · 부서 자체평가</small></span>
    </summary>
    <div className="department-report-body" id={`department-report-${id}`}>
      <section className="department-report-summary"><h4>부서 종합</h4><p>{report.summary}</p></section>
      <section className="department-evidence-audit" aria-label={`${report.departmentName} 근거 충족도 진단`}>
        <h4>근거 충족도 진단</h4>
        <dl><div><dt>부서 자체평가</dt><dd>{report.confidencePercent}%</dd></div><div><dt>고유 근거 ID</dt><dd>{uniqueEvidenceIds.size}개</dd></div><div><dt>근거가 있는 직원</dt><dd>{evidencedRoleCount}/{report.roleFindings.length}명</dd></div><div><dt>근거 공백이 남은 직원</dt><dd>{gapRoleCount}명</dd></div></dl>
        <p>이 수치는 상승 확률이나 모델 신뢰도가 아닙니다. 직원이 인용한 근거의 공식성·최신성·교차검증·시점 정합성에 대한 부서 자체평가이며, 아래 근거 ID와 공백을 함께 확인해야 합니다.</p>
      </section>
      <section><h4>직원별 상세 분석</h4><div className="department-role-reports">
        {report.roleFindings.map((item) => <article key={item.agentId}>
          <header><strong>{item.role}</strong><code>{item.agentId}</code></header>
          <p>{item.finding}</p>
          {item.evidenceIds.length > 0 && <div className="department-report-evidence"><b>근거 ID</b><span>{item.evidenceIds.join(" · ")}</span></div>}
          {item.counterevidence.length > 0 && <div className="department-report-counter"><b>반대 근거</b><ul>{item.counterevidence.map((counter, index) => <li key={`${index}-${counter}`}>{counter}</li>)}</ul></div>}
          {item.evidenceGap && <div className="department-report-gap"><b>근거 공백</b><p>{item.evidenceGap}</p></div>}
        </article>)}
      </div></section>
      <div className="department-report-columns">
        <section><h4>위험·반대 조건</h4>{report.risks.length ? <ul>{report.risks.map((risk, index) => <li key={`${index}-${risk}`}>{risk}</li>)}</ul> : <p>별도로 기록된 위험이 없습니다.</p>}</section>
        <section><h4>추가 확인·후속 조치</h4>{report.nextActions.length ? <ol>{report.nextActions.map((action, index) => <li key={`${index}-${action}`}>{action}</li>)}</ol> : <p>별도로 기록된 후속 조치가 없습니다.</p>}</section>
      </div>
    </div>
  </details>;
}

function PortfolioCharts({ charts }: { charts?: TechnicalChartEvidence[] }) {
  if (!charts?.length) return <section className="portfolio-chart-evidence-empty"><span>PORTFOLIO CHARTS</span><h3>종목별 분석 차트</h3><p>분석 당시 완료 OHLCV가 20봉 미만이거나 가격 스냅샷을 만들지 못해 선이 그어진 차트를 보존하지 못했습니다.</p></section>;
  return <section className="portfolio-chart-evidence-list" aria-labelledby="portfolio-chart-evidence-heading">
    <header><span>PORTFOLIO CHARTS · POINT IN TIME</span><h3 id="portfolio-chart-evidence-heading">보유종목별 차트와 관측선</h3><p>각 차트는 분석 당시 완료 봉으로 고·저점, 저점 연결 추세선과 최근 가격 범위를 다시 계산하지 않고 보존한 자료입니다.</p></header>
    {charts.map((chart) => <TechnicalChartEvidenceView key={`${chart.sourceSnapshotId}-${chart.symbol}`} evidence={chart} />)}
  </section>;
}

function TelegramEvidenceStatus({ value, hasDetailedTrace }: { value: NonNullable<AnalysisDetail["record"]["telegramEvidence"]>; hasDetailedTrace: boolean }) {
  const candidateCount = value.snapshotCandidateCount ?? value.includedCount;
  return <section className="analysis-evidence-status" aria-label="Telegram 근거 연결 상태">
    <div><span>TELEGRAM EVIDENCE</span><strong>{hasDetailedTrace ? value.includedCount > 0 ? "보고 인용됨" : "보고 인용 0건" : "과거 기록 · 인용 여부 미분리"}</strong></div>
    <dl>
      <div><dt>기간 내 저장</dt><dd>{value.totalAvailableCount}건</dd></div>
      <div><dt>분석 후보</dt><dd>{candidateCount}건</dd></div>
      <div><dt>{hasDetailedTrace ? "직원 조회 / 보고 인용" : "과거 포함 표기"}</dt><dd>{hasDetailedTrace ? `${value.retrievedCount ?? 0} / ${value.includedCount}건` : `${value.includedCount}건`}</dd></div>
      <div><dt>선택 채널</dt><dd>{value.selectedSourceCount}개</dd></div>
    </dl>
    <p>{value.syncStatus} · {value.message}{!hasDetailedTrace ? " · 이 기록은 구버전이라 후보 수와 실제 인용 수를 구분할 수 없습니다." : ""}</p>
  </section>;
}

function ExternalEvidenceSources({ sources }: { sources?: ExternalEvidenceSource[] }) {
  if (!sources) return null;
  const citedCount = sources.filter((source) => source.cited).length;
  return <section className="external-evidence-sources" aria-labelledby="external-evidence-source-heading">
    <header><span>SOURCE PROVENANCE</span><h3 id="external-evidence-source-heading">뉴스·Telegram·공시 원문 계보</h3><p>직원이 실제로 조회한 자료와 보고서에 인용한 자료를 구분합니다. 외부 내용은 지시가 아닌 근거로만 취급합니다.</p></header>
    <p className="external-evidence-summary">조회 {sources.length}건 · 보고 인용 {citedCount}건</p>
    {sources.length ? <ul>{sources.map((source) => <li key={source.evidenceId}>
      <div><span data-provider={source.medium}>{source.medium === "news" ? "네이버 뉴스" : source.medium === "telegram" ? "Telegram" : "OpenDART"}</span><strong>{source.cited ? "보고 인용" : "조회만 함"}</strong></div>
      <h4>{source.title}</h4>
      <p>{source.sourceName} · {source.publishedAt ?? "발행 시각 미기록"}</p>
      <small>{source.sourceUrl ?? "공개 원문 URL 없음"}</small>
      {source.platformUrl && <small>네이버 제공 링크: {source.platformUrl}</small>}
      <code>{source.evidenceId}</code>
    </li>)}</ul> : <p>이 분석에서 직원이 조회한 네이버 뉴스·Telegram·OpenDART 원문이 없습니다.</p>}
  </section>;
}

export function AnalysisWorkspace({ refreshToken, onAnalyzeCandidate }: { refreshToken: number; onAnalyzeCandidate?: (candidate: MarketScreenerCandidate) => void }) {
  const [records, setRecords] = useState<AnalysisSummary[]>([]);
  const [filter, setFilter] = useState<AnalysisFilter>("all");
  const [classification, setClassification] = useState<AnalysisClassification>("all");
  const [selectedDate, setSelectedDate] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<AnalysisDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cloneDraft, setCloneDraft] = useState<CloneDraft | null>(null);
  const [cloneRunning, setCloneRunning] = useState(false);
  const [comparison, setComparison] = useState<ExperimentComparison | null>(null);
  const [walkForwardFolds, setWalkForwardFolds] = useState(3);
  const [walkForwardRunning, setWalkForwardRunning] = useState(false);
  const [walkForward, setWalkForward] = useState<WalkForwardReport | null>(null);
  const [walkForwardHistory, setWalkForwardHistory] = useState<WalkForwardReport[]>([]);
  const [forecastTraces, setForecastTraces] = useState<ForecastTrace[]>([]);
  const [forecastError, setForecastError] = useState<string | null>(null);
  const [screenerMarket, setScreenerMarket] = useState<"kr" | "us">("kr");
  const [screenerPreset, setScreenerPreset] = useState<MarketScreenerPreset>("balanced");
  const [screenerRunning, setScreenerRunning] = useState(false);
  const [screenerResult, setScreenerResult] = useState<MarketScreenerResult | null>(null);
  const [screenerError, setScreenerError] = useState<string | null>(null);

  const loadForecasts = async () => {
    try {
      setForecastTraces(await invoke<ForecastTrace[]>("probability_forecast_history", { limit: 20 }));
      setForecastError(null);
    } catch (loadError) {
      setForecastError(String(loadError));
    }
  };

  const loadRecords = async () => {
    setLoading(true);
    try {
      const next = await invoke<AnalysisSummary[]>("analysis_record_history", { limit: 100 });
      setRecords(next);
      setSelectedId((current) => current && next.some((record) => record.recordId === current) ? current : next[0]?.recordId ?? null);
      setError(null);
    } catch (loadError) {
      setError(String(loadError));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { void loadRecords(); void loadForecasts(); }, [refreshToken]);

  const filteredRecords = useMemo(() => records.filter((record) => {
    const matchesCategory = filter === "all" || filter === "strategy" && record.kind === "strategy" || filter === record.market;
    const matchesClassification = classification === "all" || record.classification === classification;
    return matchesCategory && matchesClassification && (!selectedDate || dateKey(record.completedAtMs) === selectedDate);
  }), [classification, filter, records, selectedDate]);

  const filteredForecastTraces = useMemo(() => forecastTraces.filter(({ forecast }) => {
    const matchesCategory = filter === "all" || filter !== "strategy" && forecastAssetMarket(forecast.assetClass) === filter;
    return matchesCategory && (!selectedDate || dateKey(forecast.createdAtMs) === selectedDate);
  }), [filter, forecastTraces, selectedDate]);

  useEffect(() => {
    if (selectedId && filteredRecords.some((record) => record.recordId === selectedId)) return;
    setSelectedId(filteredRecords[0]?.recordId ?? null);
  }, [filteredRecords, selectedId]);

  useEffect(() => {
    if (!selectedId) { setDetail(null); return; }
    let active = true;
    setDetailLoading(true);
    invoke<AnalysisDetail>("analysis_record_detail", { recordId: selectedId })
      .then((next) => { if (active) { setDetail(next); setError(null); } })
      .catch((loadError) => { if (active) setError(String(loadError)); })
      .finally(() => { if (active) setDetailLoading(false); });
    return () => { active = false; };
  }, [selectedId]);

  useEffect(() => {
    const config = detail?.record.config;
    const candidate = detail?.record.report?.strategyCandidate as ({ entrySignal?: { type?: string; fastWindow?: number; slowWindow?: number } } | undefined);
    if (!config || candidate?.entrySignal?.type !== "moving_average_cross") { setCloneDraft(null); setComparison(null); setWalkForward(null); setWalkForwardHistory([]); return; }
    setCloneDraft({
      fastWindow: String(candidate.entrySignal.fastWindow ?? 5), slowWindow: String(candidate.entrySignal.slowWindow ?? 20),
      initialCashMinor: String(config.initialCashMinor), orderQuantity: String(config.orderQuantity),
      buyFeeBps: String(config.costs.buyFeeBps), sellFeeBps: String(config.costs.sellFeeBps),
      sellTaxBps: String(config.costs.sellTaxBps), slippageBps: String(config.costs.slippageBps),
      stopLossBps: config.riskLimits ? String(config.riskLimits.stopLossBps) : "",
      takeProfitBps: config.riskLimits ? String(config.riskLimits.takeProfitBps) : "",
      dailyLossLimitMinor: config.riskLimits ? String(config.riskLimits.dailyLossLimitMinor) : "",
      closeOpenPositionAtEnd: config.closeOpenPositionAtEnd,
    });
    setComparison(null);
    setWalkForward(null);
  }, [detail]);

  useEffect(() => {
    const experimentId = detail?.record.type === "strategy_review" ? detail.record.config?.experimentId : undefined;
    if (!experimentId) return;
    let cancelled = false;
    void invoke<WalkForwardReport[]>("backtest_experiment_walk_forward_history", { sourceExperimentId: experimentId })
      .then((reports) => { if (!cancelled) { setWalkForwardHistory(reports); setWalkForward(reports[0] ?? null); } })
      .catch(() => { if (!cancelled) { setWalkForwardHistory([]); setWalkForward(null); } });
    return () => { cancelled = true; };
  }, [detail]);

  const updateCloneDraft = (field: keyof CloneDraft, value: string | boolean) => setCloneDraft((current) => current ? { ...current, [field]: value } : current);
  const runClone = async () => {
    if (!cloneDraft || !detail?.record.config) return;
    const riskValues = [cloneDraft.stopLossBps, cloneDraft.takeProfitBps, cloneDraft.dailyLossLimitMinor];
    const riskEnabled = riskValues.every((value) => value.trim() !== "");
    if (!riskEnabled && riskValues.some((value) => value.trim() !== "")) { setError("손절·익절·일일손실 한도는 모두 입력하거나 모두 비워야 합니다."); return; }
    setCloneRunning(true);
    try {
      const next = await invoke<ExperimentComparison>("backtest_experiment_clone_run", { request: {
        sourceExperimentId: detail.record.config.experimentId,
        fastWindow: Number(cloneDraft.fastWindow), slowWindow: Number(cloneDraft.slowWindow),
        initialCashMinor: Number(cloneDraft.initialCashMinor), orderQuantity: Number(cloneDraft.orderQuantity),
        closeOpenPositionAtEnd: cloneDraft.closeOpenPositionAtEnd,
        buyFeeBps: Number(cloneDraft.buyFeeBps), sellFeeBps: Number(cloneDraft.sellFeeBps),
        sellTaxBps: Number(cloneDraft.sellTaxBps), slippageBps: Number(cloneDraft.slippageBps),
        stopLossBps: riskEnabled ? Number(cloneDraft.stopLossBps) : null,
        takeProfitBps: riskEnabled ? Number(cloneDraft.takeProfitBps) : null,
        dailyLossLimitMinor: riskEnabled ? Number(cloneDraft.dailyLossLimitMinor) : null,
      } });
      setComparison(next);
      await loadRecords();
      setError(null);
    } catch (runError) { setError(String(runError)); }
    finally { setCloneRunning(false); }
  };
  const runWalkForward = async () => {
    if (!detail?.record.config) return;
    setWalkForwardRunning(true);
    try {
      const report = await invoke<WalkForwardReport>("backtest_experiment_walk_forward", { request: {
        sourceExperimentId: detail.record.config.experimentId,
        foldCount: walkForwardFolds,
      } });
      setWalkForward(report);
      setWalkForwardHistory((current) => [report, ...current.filter((item) => item.validationRunId !== report.validationRunId)].slice(0, 20));
      setError(null);
    } catch (runError) { setError(String(runError)); }
    finally { setWalkForwardRunning(false); }
  };
  const runMarketScreener = async () => {
    setScreenerRunning(true);
    setScreenerError(null);
    try {
      const next = await invoke<MarketScreenerResult>("toss_market_screener", { request: {
        market: screenerMarket, preset: screenerPreset, technicalCandidateCount: 12, resultCount: 5,
      } });
      if (next.liveOrderEnabled) throw new Error("후보 탐색 응답의 실주문 잠금 계약이 손상되었습니다.");
      setScreenerResult(next);
    } catch (runError) {
      setScreenerResult(null);
      setScreenerError(String(runError));
    } finally {
      setScreenerRunning(false);
    }
  };

  return <main className="analysis-workspace">
    <header className="analysis-header">
      <div><p className="eyebrow">LOCAL ANALYSIS VAULT</p><h2>전략 검증·종목 분석 기록</h2><p>검증에 사용한 가격대와 요청·완료 시각을 함께 보존합니다.</p></div>
      <button type="button" onClick={() => { void loadRecords(); void loadForecasts(); }} disabled={loading}>{loading ? "불러오는 중" : "기록 새로고침"}</button>
    </header>
    <div className="analysis-layout">
      <aside className="analysis-browser" aria-label="분석 기록 선택">
        <div className="analysis-filters" aria-label="기록 분류">
          {FILTERS.map((item) => <button key={item.id} type="button" className={filter === item.id ? "is-active" : ""} aria-pressed={filter === item.id} onClick={() => setFilter(item.id)}>{item.label}</button>)}
        </div>
        <label className="analysis-date-filter">분석 성격<select value={classification} onChange={(event) => setClassification(event.currentTarget.value as AnalysisClassification)}><option value="all">전체 성격</option><option value="system_check">시스템 검사</option><option value="research_experiment">연구 실험</option><option value="promotion_candidate">승격 후보</option></select></label>
        <label className="analysis-date-filter">분석 완료일<input type="date" value={selectedDate} onChange={(event) => setSelectedDate(event.currentTarget.value)} /></label>
        {selectedDate && <button className="analysis-date-clear" type="button" onClick={() => setSelectedDate("")}>전체 날짜 보기</button>}
        <div className="analysis-list-heading"><strong>{FILTERS.find((item) => item.id === filter)?.label}</strong><span>{filteredRecords.length}건</span></div>
        {error && !records.length ? <div className="analysis-state" role="alert"><strong>기록을 불러오지 못했습니다.</strong><p>{error}</p><button type="button" onClick={() => void loadRecords()}>다시 시도</button></div>
          : loading && !records.length ? <div className="analysis-state"><strong>로컬 기록 확인 중</strong><p>SQLite 무결성과 저장된 분석을 읽고 있습니다.</p></div>
            : filteredRecords.length === 0 ? <div className="analysis-state"><strong>조건에 맞는 기록이 없습니다.</strong><p>퀀트 논문 연구원의 전략 검증이 완료되면 자동으로 저장됩니다.</p></div>
              : <ul className="analysis-record-list">{filteredRecords.map((record) => <li key={record.recordId}>
                <button type="button" className={selectedId === record.recordId ? "is-active" : ""} onClick={() => setSelectedId(record.recordId)}>
                  <span><b>{MARKET_LABELS[record.market]} · {CLASSIFICATION_LABELS[record.classification]}</b><time dateTime={new Date(record.completedAtMs).toISOString()}>{new Date(record.completedAtMs).toLocaleDateString("ko-KR")}</time></span>
                  <strong>{record.title}</strong><small>{record.symbol || "종목 미지정"} · {record.priceLowMinor != null && record.priceHighMinor != null && record.currency ? `${formatMoney(record.priceLowMinor, record.currency)}~${formatMoney(record.priceHighMinor, record.currency)}` : statusLabel(record.status)}</small>
                </button>
              </li>)}</ul>}
      </aside>
      <section className="analysis-detail" aria-live="polite">
        <details className="market-screener-panel">
          <summary><span>MARKET CANDIDATE PIPELINE</span><strong>시장 후보 탐색</strong><small>{screenerResult ? `${screenerResult.candidates.length}개 후보 · ${formatDateTime(screenerResult.asOfMs)}` : "토스 읽기 전용"}</small></summary>
          <div className="market-screener-controls">
            <label>시장<select value={screenerMarket} onChange={(event) => setScreenerMarket(event.currentTarget.value as "kr" | "us")} disabled={screenerRunning}><option value="kr">국장</option><option value="us">미장</option></select></label>
            <label>탐색 기준<select value={screenerPreset} onChange={(event) => setScreenerPreset(event.currentTarget.value as MarketScreenerPreset)} disabled={screenerRunning}><option value="balanced">균형</option><option value="momentum">추세</option><option value="reversal">반전 관찰</option></select></label>
            <button type="button" onClick={() => void runMarketScreener()} disabled={screenerRunning}>{screenerRunning ? "랭킹·일봉 검토 중" : "후보 탐색 실행"}</button>
          </div>
          <p>시장 랭킹을 저비용 1차 필터로 쓰고 상위 12종목만 일봉 기술 조건을 검토합니다. 결과는 추천·주문 승인이 아닙니다.</p>
          {screenerError && <p className="analysis-inline-error" role="alert">후보를 탐색하지 못했습니다. {screenerError}</p>}
          {screenerResult && <>
            <div className="market-screener-meta"><span>1차 유니버스 {screenerResult.coarseUniverseCount}개</span><span>기술 검토 {screenerResult.technicalEvaluatedCount}개</span><span>조건 제외 {screenerResult.excludedCount}개</span><span>부분 실패 {screenerResult.errors.length}개</span></div>
            {screenerResult.candidates.length === 0 ? <p className="market-screener-empty">현재 조건을 모두 통과한 후보가 없습니다. 조건을 자동 완화하지 않습니다.</p> : <ol className="market-screener-results">{screenerResult.candidates.map((candidate) => <li key={candidate.symbol}>
              <div><b>{candidate.name}</b><code>{candidate.symbol} · {candidate.market}</code></div>
              <dl><div><dt>현재가</dt><dd>{formatMoney(candidate.latestPriceMinor, candidate.currency)}</dd></div><div><dt>20일</dt><dd>{formatPercent(candidate.twentyDayReturnBps)}</dd></div><div><dt>RSI</dt><dd>{(candidate.rsi14Bps / 100).toFixed(1)}</dd></div><div><dt>1차 점수</dt><dd>{formatPercent(candidate.coarseScoreBps)}</dd></div></dl>
              <small>{candidate.reasons.join(" · ")}</small>
              {onAnalyzeCandidate && <button type="button" onClick={() => onAnalyzeCandidate(candidate)}>이 종목 분석 안건 만들기</button>}
            </li>)}</ol>}
            <ul className="market-screener-warnings">{screenerResult.warnings.map((warning) => <li key={warning}>{warning}</li>)}{screenerResult.errors.map((item) => <li key={item}>부분 실패: {item}</li>)}</ul>
          </>}
        </details>
        <details className="forecast-trace-panel">
          <summary><span>PROBABILITY FORECAST TRACE</span><strong>예측 기반시설</strong><small>{filteredForecastTraces.length ? `${filteredForecastTraces.length}개 불변 기록` : "조건에 맞는 기록 없음"}</small></summary>
          {forecastError ? <p className="analysis-inline-error">예측 기록을 불러오지 못했습니다. {forecastError}</p>
            : filteredForecastTraces.length === 0 ? <p>선택한 시장·날짜에 저장된 검증 모델 예측이 없습니다. 확률을 임의로 생성하지 않으며, 모델·데이터셋·horizon 식별자가 모두 있는 결과만 표시합니다.</p>
              : <ul>{filteredForecastTraces.map(({ forecast, calibration }) => <li key={forecast.forecast.forecastId}>
                <div><b>{forecastAssetLabel[forecast.assetClass]} · {evidenceModeLabel[forecast.evidenceMode]}</b><time dateTime={new Date(forecast.createdAtMs).toISOString()}>{formatDateTime(forecast.createdAtMs)}</time></div>
                <strong>{forecast.forecast.modelId}@{forecast.forecast.modelVersion}</strong>
                <small>dataset {forecast.forecast.datasetId} · asset {forecast.forecast.assetContractId} · horizon {forecast.forecast.horizonMs.toLocaleString("ko-KR")}ms</small>
                <span>{forecast.forecast.upProbabilityBps == null ? forecast.forecast.unavailableReason ?? "확률 산출 불가" : `상승 ${formatPercent(forecast.forecast.upProbabilityBps)} · 하락 ${formatPercent(forecast.forecast.downProbabilityBps)} · 신뢰도 ${formatPercent(forecast.forecast.modelReliabilityBps)}`}{calibration ? ` · 보정 ${calibration.metrics.sampleCount}표본 / ECE ${formatPercent(calibration.metrics.expectedCalibrationErrorBps)}` : " · 자산군별 보정 미실행"}</span>
              </li>)}</ul>}
        </details>
        {detailLoading ? <div className="analysis-detail-empty"><strong>분석 결과 불러오는 중</strong><p>선택한 불변 기록을 확인하고 있습니다.</p></div>
          : !detail ? <div className="analysis-detail-empty"><strong>분석 기록을 선택하세요.</strong><p>왼쪽 목록에서 전략 또는 종목 분석을 선택하면 상세 결과가 표시됩니다.</p></div>
            : detail.record.type === "meeting" ? <>
              <header><div><span>{MARKET_LABELS[detail.summary.market]} · 회의 종합</span><h2>{detail.summary.title}</h2><p>{detail.summary.symbol || "종목 미지정"}</p></div><strong className={detail.summary.status === "completed" ? "is-ready" : "is-blocked"}>{statusLabel(detail.summary.status)}</strong></header>
              <dl className="analysis-meta"><div><dt>분석 요청</dt><dd>{formatDateTime(detail.summary.requestedAtMs)}</dd></div><div><dt>분석 완료</dt><dd>{formatDateTime(detail.summary.completedAtMs)}</dd></div></dl>
              {detail.record.portfolio && <PortfolioOverview snapshot={detail.record.portfolio} title="분석 당시 보유자산" />}
              {(detail.record.portfolio || detail.record.portfolioCharts?.length) && <PortfolioCharts charts={detail.record.portfolioCharts} />}
              {detail.record.telegramEvidence && <TelegramEvidenceStatus value={detail.record.telegramEvidence} hasDetailedTrace={Boolean(detail.record.evidenceSources)} />}
              <ExternalEvidenceSources sources={detail.record.evidenceSources} />
              <section className="analysis-request"><span>AGENDA</span><h3>회의 안건</h3><p>{detail.record.topic}</p></section>
              <section className="analysis-result-section"><span>DECISION</span><h3>{detail.record.synthesis?.decision ?? "결정 미기록"}</h3><p>{detail.record.synthesis?.summary}</p></section>
              <section className="meeting-department-report-section" aria-labelledby="meeting-department-report-heading">
                <header><span>REPORTS · FULL TEXT</span><h3 id="meeting-department-report-heading">부서별 상세 보고</h3><p>부서 종합과 직원별 근거·반대 근거·공백을 함께 보존합니다. 각 보고는 접어서 비교할 수 있습니다.</p></header>
                <div className="meeting-department-reports">
                  {Object.entries(detail.record.reports ?? {}).length
                    ? Object.entries(detail.record.reports ?? {}).map(([id, report]) => <DepartmentReportPanel key={id} id={id} report={report} />)
                    : <p className="meeting-department-report-empty">저장된 부서 보고가 없습니다.</p>}
                </div>
              </section>
              <div className="analysis-result-grid"><section><span>CONSENSUS</span><h3>부서 합의</h3><ul>{(detail.record.synthesis?.consensus ?? []).map((item, index) => <li key={`${index}-${item}`}>{item}</li>)}</ul></section><section><span>CONDITIONS</span><h3>조건·이견</h3><ul>{[...(detail.record.synthesis?.conditions ?? []), ...(detail.record.synthesis?.disagreements ?? [])].map((item, index) => <li key={`${index}-${item}`}>{item}</li>)}</ul></section></div>
            </> : detail.record.type === "department_delegation" ? <>
              <header><div><span>CODEX · 승인형 부서 종합</span><h2>{detail.summary.title}</h2><p>{detail.record.topic}</p></div><strong className={detail.record.departmentReport?.conclusion === "proceed" ? "is-ready" : "is-blocked"}>{detail.record.departmentReport?.conclusion ?? "미기록"}</strong></header>
              <dl className="analysis-meta"><div><dt>업무 요청</dt><dd>{formatDateTime(detail.summary.requestedAtMs)}</dd></div><div><dt>종합 완료</dt><dd>{formatDateTime(detail.summary.completedAtMs)}</dd></div><div><dt>부서 자체평가</dt><dd>{detail.record.departmentReport?.confidencePercent ?? 0}%</dd></div></dl>
              {detail.record.portfolio && <PortfolioOverview snapshot={detail.record.portfolio} title="부서 분석 당시 보유자산" />}
              {(detail.record.portfolio || detail.record.portfolioCharts?.length) && <PortfolioCharts charts={detail.record.portfolioCharts} />}
              {detail.record.telegramEvidence && <TelegramEvidenceStatus value={detail.record.telegramEvidence} hasDetailedTrace={Boolean(detail.record.evidenceSources)} />}
              <ExternalEvidenceSources sources={detail.record.evidenceSources} />
              {detail.record.departmentReport
                ? <section className="meeting-department-report-section" aria-label="부서 상세 보고"><DepartmentReportPanel id={detail.summary.recordId} report={detail.record.departmentReport} /></section>
                : <p className="meeting-department-report-empty">저장된 부서 보고가 없습니다.</p>}
              <p className="analysis-inline-error">사용자가 승인한 부서 내부 종합입니다. 본부장 회의나 주문 후보로 자동 승격되지 않습니다.</p>
            </> : detail.record.type === "role_report" ? <>
              <header><div><span>CODEX · 개별 역할 소견</span><h2>{detail.summary.title}</h2><p>{detail.record.report?.scope}</p></div><strong className="is-ready">근거 충족도 {detail.record.report?.confidencePercent ?? 0}%</strong></header>
              <dl className="analysis-meta"><div><dt>소견 요청</dt><dd>{formatDateTime(detail.summary.requestedAtMs)}</dd></div><div><dt>소견 완료</dt><dd>{formatDateTime(detail.summary.completedAtMs)}</dd></div><div><dt>담당 역할</dt><dd>{detail.record.report?.role ?? "역할 미기록"}</dd></div><div><dt>관점</dt><dd>{detail.record.report?.stance ?? "미기록"}</dd></div></dl>
              <section className="analysis-result-section"><span>ROLE-ONLY SUMMARY</span><h3>개별 소견</h3><p>{detail.record.report?.summary}</p></section>
              <div className="analysis-result-grid"><section><span>FINDINGS</span><h3>역할 한정 결과</h3><ul>{detail.record.report?.findings?.map((item, index) => <li key={`${index}-${item}`}>{item}</li>)}</ul></section><section><span>BOUNDARIES</span><h3>근거 공백·추가 요청</h3><ul>{[...(detail.record.report?.evidenceGaps ?? []), ...(detail.record.report?.nextRequests ?? [])].map((item, index) => <li key={`${index}-${item}`}>{item}</li>)}</ul></section></div>
              <section className="analysis-result-section"><span>EVIDENCE TRACE</span><h3>근거·리비전·반대 근거</h3><ul>{detail.record.report?.evidence?.map((item) => <li key={item.evidenceId}><strong>{item.evidenceId} · {item.source ?? "출처 미기록"}</strong><small>{item.sourceRevision ? `rev ${item.sourceRevision} · ` : ""}{item.observation ?? "관측 미기록"}{item.counterevidence?.length ? ` · 반대 근거: ${item.counterevidence.join(" · ")}` : ""}</small></li>)}</ul></section>
              <ExternalEvidenceSources sources={detail.record.evidenceSources} />
              {detail.record.chartEvidence && <TechnicalChartEvidenceView evidence={detail.record.chartEvidence} />}
              <p className="analysis-inline-error">이 기록은 해당 직원의 독립 소견입니다. 전체 분석, 최종 투자 판단 또는 주문 후보가 아닙니다.</p>
            </> : <>
              <header><div><span>{MARKET_LABELS[detail.summary.market]} · 전략 검증</span><h2>{detail.summary.title}</h2><p>{detail.summary.symbol} · {detail.summary.currency} · {detail.record.provider ?? "계약 검토"}</p></div><strong className={detail.record.review?.executable ? "is-ready" : "is-blocked"}>{detail.record.review?.executable ? "검증 완료" : "실행 차단"}</strong></header>
              <dl className="analysis-meta">
                <div><dt>분석 요청</dt><dd>{formatDateTime(detail.summary.requestedAtMs)}</dd></div>
                <div><dt>분석 완료</dt><dd>{formatDateTime(detail.summary.completedAtMs)}</dd></div>
                <div><dt>검증 가격대</dt><dd>{detail.summary.priceLowMinor != null && detail.summary.priceHighMinor != null ? `${formatMoney(detail.summary.priceLowMinor, detail.summary.currency)} ~ ${formatMoney(detail.summary.priceHighMinor, detail.summary.currency)}` : "가격 데이터 없음"}</dd></div>
                <div><dt>데이터 조건</dt><dd>{detail.record.result ? `${detail.record.interval} · ${detail.record.result.inputBarCount}봉 · ${detail.record.adjusted ? "수정주가" : "원주가"}` : "계약 검토 단계에서 차단"}</dd></div>
              </dl>
              <section className="analysis-request"><span>REQUEST</span><h3>요청 내용</h3><p>{detail.record.report?.request}</p></section>
              <section className="analysis-metrics" aria-label="전략 검증 지표">
                <div><span>총수익률</span><strong className={(detail.summary.totalReturnBps ?? 0) >= 0 ? "is-positive" : "is-negative"}>{formatPercent(detail.summary.totalReturnBps)}</strong></div>
                <div><span>최대 낙폭</span><strong>{formatPercent(detail.summary.maxDrawdownBps)}</strong></div>
                <div><span>승률</span><strong>{formatPercent(detail.summary.winRateBps)}</strong></div>
                <div><span>완료 거래</span><strong>{detail.summary.completedTradeCount == null ? "계산 불가" : `${detail.summary.completedTradeCount}회`}</strong></div>
              </section>
              <section className="analysis-result-section"><span>HYPOTHESIS</span><h3>분석 가설</h3><p>{detail.record.report?.strategyCandidate?.hypothesis}</p></section>
              {detail.record.result?.performance && <div className="analysis-result-grid">
                <section><span>RISK-ADJUSTED</span><h3>위험조정 성과</h3><ul>
                  <li><strong>Sharpe</strong><small>{formatRatio(detail.record.result.performance.sharpeRatioMilli)}</small></li>
                  <li><strong>Sortino</strong><small>{formatRatio(detail.record.result.performance.sortinoRatioMilli)}</small></li>
                  <li><strong>연환산 변동성</strong><small>{formatPercent(detail.record.result.performance.annualizedVolatilityBps)}</small></li>
                </ul></section>
                <section><span>PRICE BENCHMARK</span><h3>동일 종목 보유 대비</h3><ul>
                  <li><strong>가격 수익률</strong><small>{formatPercent(detail.record.result.performance.priceBenchmarkReturnBps)}</small></li>
                  <li><strong>전략 알파</strong><small>{formatPercent(detail.record.result.performance.alphaVsPriceBenchmarkBps)}</small></li>
                </ul><p>별도 시장지수가 아니라 같은 기간 해당 종목 단순 가격 수익률과 비교합니다.</p></section>
              </div>}
              {detail.record.result?.patternProbability && <section className="analysis-result-section"><span>CONDITIONAL STATISTICS</span><h3>연속 봉·볼린저 조건부 통계</h3>
                <p>현재 {directionLabel(detail.record.result.patternProbability.currentSequenceDirection)} {detail.record.result.patternProbability.currentSequenceCount}개 연속 · 동일 패턴 {detail.record.result.patternProbability.nextCandle.sampleSize}회</p>
                <ul>
                  <li>다음 봉: 양봉 {formatPercent(detail.record.result.patternProbability.nextCandle.bullishProbabilityBps)} · 음봉 {formatPercent(detail.record.result.patternProbability.nextCandle.bearishProbabilityBps)} · 도지 {formatPercent(detail.record.result.patternProbability.nextCandle.dojiProbabilityBps)}</li>
                  {detail.record.result.patternProbability.horizonOutcomes.map((item) => <li key={item.horizonBars}>{item.horizonBars}봉 후: 상승 {formatPercent(item.positiveProbabilityBps)} · 하락 {formatPercent(item.negativeProbabilityBps)} · 표본 {item.sampleSize}회</li>)}
                </ul>
                <p>최소 {detail.record.result.patternProbability.minimumPublishedSample}회 미만 표본은 확률을 숨깁니다. 이 통계는 예측 모델이나 주문 승인이 아닙니다.</p>
              </section>}
              {cloneDraft && detail.record.config && <section className="experiment-clone-panel">
                <span>IMMUTABLE EXPERIMENT FORK</span><h3>저장 실험 복제·재실행</h3>
                <p>원본과 가격 데이터셋은 그대로 두고 변경한 조건을 새 연구 실험 ID로 저장합니다.</p>
                <div className="experiment-clone-grid">
                  {([["fastWindow", "빠른 이평"], ["slowWindow", "느린 이평"], ["initialCashMinor", "초기자금(최소단위)"], ["orderQuantity", "주문 수량"], ["buyFeeBps", "매수 수수료(bp)"], ["sellFeeBps", "매도 수수료(bp)"], ["sellTaxBps", "매도 세금(bp)"], ["slippageBps", "슬리피지(bp)"], ["stopLossBps", "손절(bp, 선택)"], ["takeProfitBps", "익절(bp, 선택)"], ["dailyLossLimitMinor", "일일손실 한도(선택)"]] as Array<[keyof CloneDraft, string]>).map(([field, label]) => <label key={field}>{label}<input type="number" min="0" step={field.includes("Fee") || field.includes("Tax") || field === "slippageBps" ? "0.001" : "1"} value={String(cloneDraft[field])} onChange={(event) => updateCloneDraft(field, event.currentTarget.value)} /></label>)}
                  <label className="experiment-clone-check"><input type="checkbox" checked={cloneDraft.closeOpenPositionAtEnd} onChange={(event) => updateCloneDraft("closeOpenPositionAtEnd", event.currentTarget.checked)} />마지막 봉에서 잔여 포지션 청산</label>
                </div>
                <button type="button" onClick={() => void runClone()} disabled={cloneRunning}>{cloneRunning ? "재실행 중" : "복제본 백테스트 실행"}</button>
              </section>}
              {comparison && <section className="experiment-comparison" aria-live="polite"><span>COMPARISON</span><h3>원본과 복제본 비교</h3><div>
                {[{ label: "원본", id: comparison.sourceExperimentId, result: comparison.sourceResult }, { label: "복제본 · 연구 실험", id: comparison.clonedExperimentId, result: comparison.clonedResult }].map((item) => <article key={item.id}><b>{item.label}</b><small>{item.id}</small><dl><div><dt>총수익률</dt><dd>{formatPercent(item.result.totalReturnBps)}</dd></div><div><dt>최대 낙폭</dt><dd>{formatPercent(item.result.maxDrawdownBps)}</dd></div><div><dt>승률</dt><dd>{formatPercent(item.result.winRateBps)}</dd></div><div><dt>실현손익</dt><dd>{formatMoney(item.result.realizedPnlMinor ?? 0, detail.summary.currency)}</dd></div></dl></article>)}
              </div><p>복제본은 자동으로 승격되지 않습니다. 비교 결과를 확인한 뒤 별도 검토하세요.</p></section>}
              {detail.record.result && detail.record.config && <section className="experiment-clone-panel">
                <span>TIME-SERIES VALIDATION</span><h3>시간순 OOS 반복 검증</h3>
                <p>앞 절반을 최초 학습 구간으로 고정하고 이후 데이터를 겹치지 않는 OOS 구간으로 나눠 같은 전략을 독립 재생합니다.</p>
                <div className="walk-forward-controls"><label>OOS 구간 수<select value={walkForwardFolds} onChange={(event) => setWalkForwardFolds(Number(event.currentTarget.value))}>{[2, 3, 4, 5].map((count) => <option key={count} value={count}>{count}개</option>)}</select></label><button type="button" onClick={() => void runWalkForward()} disabled={walkForwardRunning}>{walkForwardRunning ? "검증 중" : "OOS 검증 실행"}</button></div>
              </section>}
              {walkForward && <section className="experiment-comparison walk-forward-report" aria-live="polite"><span>OUT-OF-SAMPLE · SAVED</span><h3>시간순 검증 결과 · OOS 수익 {walkForward.positiveOosFoldCount}/{walkForward.foldCount}구간</h3>{walkForwardHistory.length > 1 && <label className="walk-forward-history-select">저장 검증 선택<select value={walkForward.validationRunId} onChange={(event) => setWalkForward(walkForwardHistory.find((item) => item.validationRunId === event.currentTarget.value) ?? walkForward)}>{walkForwardHistory.map((item) => <option key={item.validationRunId} value={item.validationRunId}>{formatDateTime(item.createdAtMs)} · {item.foldCount}구간 · {item.validationRunId}</option>)}</select></label>}<p>저장 ID {walkForward.validationRunId} · 전략 실험 {walkForward.strategyTrialCount}회 · {formatDateTime(walkForward.createdAtMs)}</p><p>최대 단일 구간 기여 {formatPercent(walkForward.largestAbsoluteOosReturnShareBps)} · 구간 수익률 범위 {formatPercent(walkForward.oosReturnSpreadBps)}</p><p className={walkForward.meetsResearchSampleMinimum ? "oos-sample-gate is-ready" : "oos-sample-gate is-blocked"}>Investa 운영 정책 · OOS 완료 거래 {walkForward.totalOosTradeCount}/{walkForward.minimumOosTradeCount}건 · {walkForward.meetsResearchSampleMinimum ? "연구 표본 최소값 충족" : "승격 검토 차단"}</p><div>
                {walkForward.folds.map((fold) => <article key={fold.foldNumber}><b>구간 {fold.foldNumber}</b><small>{new Date(fold.oosStartMs).toLocaleDateString("ko-KR")} ~ {new Date(fold.oosEndMs).toLocaleDateString("ko-KR")} · {fold.oosBarCount}봉</small><dl><div><dt>학습 수익률</dt><dd>{formatPercent(fold.training.totalReturnBps)}</dd></div><div><dt>OOS 수익률</dt><dd>{formatPercent(fold.outOfSample.totalReturnBps)}</dd></div><div><dt>OOS MDD</dt><dd>{formatPercent(fold.outOfSample.maxDrawdownBps)}</dd></div><div><dt>OOS 승률</dt><dd>{formatPercent(fold.outOfSample.winRateBps)}</dd></div><div><dt>기대손익</dt><dd>{fold.outOfSample.expectedTradePnlMinor == null ? "계산 불가" : formatMoney(fold.outOfSample.expectedTradePnlMinor, detail.summary.currency)}</dd></div><div><dt>Profit Factor</dt><dd>{formatRatio(fold.outOfSample.profitFactorMilli)}</dd></div><div><dt>회전율</dt><dd>{formatPercent(fold.outOfSample.turnoverBps)}</dd></div><div><dt>시장 노출</dt><dd>{formatPercent(fold.outOfSample.exposureBps)}</dd></div><div><dt>OOS 거래</dt><dd>{fold.outOfSample.completedTradeCount}회</dd></div></dl><ul className="regime-breakdown">{fold.regimes.map((regime) => <li key={regime.regime}><span>{regimeLabel(regime.regime)}</span><b>{regime.completedTradeCount}회 · {formatPercent(regime.winRateBps)}</b><small>{formatMoney(regime.realizedPnlMinor, detail.summary.currency)}</small></li>)}{fold.unclassifiedTradeCount > 0 && <li><span>분류 불가</span><b>{fold.unclassifiedTradeCount}회</b><small>과거 관측 부족</small></li>}</ul></article>)}
              </div><section className="oos-promotion-gate"><h4>모의운영 검토 게이트 · {walkForward.promotionEvaluation.policyVersion}</h4><p className={walkForward.promotionEvaluation.eligibleForPaperReview ? "is-ready" : "is-blocked"}>{walkForward.promotionEvaluation.eligibleForPaperReview ? "사용자 검토 가능" : "승격 검토 차단"}</p><ul>{walkForward.promotionEvaluation.checks.map((check) => <li key={check.checkId} className={check.passed ? "is-passed" : "is-failed"}><b>{check.passed ? "통과" : "미달"} · {check.label}</b><span>{check.observed} / {check.required}</span></li>)}</ul><small>{walkForward.promotionEvaluation.warning}</small></section><section className="oos-statistical-gate"><h4>과최적화 진단</h4>{walkForward.overfitDiagnostics.probabilityOfBacktestOverfittingBps == null ? <p>산출 보류 · 비교 전략 {walkForward.overfitDiagnostics.comparableStrategyCount}개</p> : <p>PBO {formatPercent(walkForward.overfitDiagnostics.probabilityOfBacktestOverfittingBps)} · 비교 전략 {walkForward.overfitDiagnostics.comparableStrategyCount}개 · 분할 {walkForward.overfitDiagnostics.evaluatedPartitionCount}개</p>}{walkForward.overfitDiagnostics.minimumTrackRecordLength != null && <p>95% 단측 MinTRL · 최소 {walkForward.overfitDiagnostics.minimumTrackRecordLength}개 기간 수익률</p>}<ul>{walkForward.overfitDiagnostics.blockers.map((blocker) => <li key={blocker}>{blocker}</li>)}</ul></section><ul>{[...walkForward.promotionBlockers, ...walkForward.warnings].map((warning) => <li key={warning}>{warning}</li>)}</ul></section>}
              <div className="analysis-result-grid">
                <section><span>EVIDENCE</span><h3>근거</h3>{detail.record.report?.evidence?.length ? <ul>{detail.record.report.evidence.map((evidence) => <li key={evidence.evidenceId}><strong>{evidence.summary}</strong><small>{evidence.sourceUrl}</small></li>)}</ul> : <p>저장된 근거가 없습니다.</p>}</section>
                <section><span>LIMITS</span><h3>한계·경고</h3><ul>{[...(detail.record.report?.strategyCandidate?.limitations ?? []), ...(detail.record.report?.strategyCandidate?.unknowns ?? []), ...(detail.record.warnings ?? [])].map((item, index) => <li key={`${index}-${item}`}>{item}</li>)}</ul></section>
              </div>
              {(detail.record.review?.issues.length ?? 0) > 0 && <section className="analysis-issues"><h3>계약 검증 문제</h3><ul>{detail.record.review?.issues.map((issue) => <li key={`${issue.field}-${issue.message}`}><strong>{issue.field}</strong>{issue.message}</li>)}</ul></section>}
              {error && <p className="analysis-inline-error" role="alert">최근 갱신 오류: {error}</p>}
            </>}
      </section>
    </div>
  </main>;
}
