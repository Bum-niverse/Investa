import { type PointerEvent, type WheelEvent, useEffect, useMemo, useRef, useState } from "react";
import { bigSales, bollinger, cci, dmi, ema, institutionalShift, macd, mfi, momentum, obv, parabolicSar, rsi, rollingRange, sma, stochastic, ultimateRsi, volumeDeltaEstimate, williamsR } from "./chartIndicators";

export type ChartBar = { periodStartMs: number; periodEndMs: number; openMinor: number; highMinor: number; lowMinor: number; closeMinor: number; volume: number; completed: boolean };
export type ChartSnapshot = { provider: string; symbol: string; currency: string; interval: "1d" | "1m"; adjusted: boolean; fetchedAtMs: number; bars: ChartBar[] };
type Drawing = { id: string; startTime: number; endTime: number; startPriceMinor: number; endPriceMinor: number };
type Anchor = Omit<Drawing, "id" | "endTime" | "endPriceMinor">;
type Series = { label: string; color: string; values: Array<number | null> };
type HoverPoint = { index: number; priceMinor: number; chartX: number; chartY: number };
type PanOrigin = { clientX: number; endIndex: number };

const formatMoney = (minor: number, currency: string) => new Intl.NumberFormat("ko-KR", { style: "currency", currency, maximumFractionDigits: currency === "KRW" ? 0 : 2 }).format(minor / (currency === "KRW" ? 1 : 100));
const points = (values: Array<number | null>, x: (index: number) => number, y: (value: number) => number) => values.map((value, index) => value == null || !Number.isFinite(value) ? null : `${x(index)},${y(value)}`).filter(Boolean).join(" ");
const lastDefined = (values: Array<number | null>) => {
  for (let index = values.length - 1; index >= 0; index -= 1) if (values[index] != null) return values[index];
  return null;
};
const storageKey = (snapshot: ChartSnapshot) => `investa:chart-drawings:v1:${snapshot.provider}:${snapshot.symbol}:${snapshot.interval}:${snapshot.adjusted ? "adjusted" : "raw"}`;

function readDrawings(key: string): Drawing[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(key) ?? "[]") as Drawing[];
    return Array.isArray(parsed) ? parsed.filter((item) => item && typeof item.id === "string" && [item.startTime, item.endTime, item.startPriceMinor, item.endPriceMinor].every(Number.isFinite)).slice(-100) : [];
  } catch { return []; }
}

function IndicatorPanel({ label, series, bars, top, height, guides = [] }: { label: string; series: Series[]; bars: ChartBar[]; top: number; height: number; guides?: number[] }) {
  const values = series.flatMap((item) => item.values.filter((value): value is number => value != null && Number.isFinite(value)));
  const minimum = Math.min(...values, ...guides);
  const maximum = Math.max(...values, ...guides);
  const range = Math.max(1, maximum - minimum);
  const y = (value: number) => top + height - 12 - ((value - minimum) / range) * (height - 24);
  const x = (index: number) => (960 / Math.max(1, bars.length)) * index + 960 / Math.max(2, bars.length * 2);
  return <g className="paper-indicator-panel">
    <rect x="0" y={top} width="960" height={height} fill="#0b110d" stroke="#26332c" />
    <text x="10" y={top + 17} fill="#aab6af" fontSize="13" fontFamily="Segoe UI, Malgun Gothic, sans-serif" fontWeight="600">{label}</text>
    {guides.map((guide) => <g key={guide}><line x1="0" x2="960" y1={y(guide)} y2={y(guide)} stroke="#334139" strokeDasharray="3 4" /><text x="952" y={y(guide) - 3} textAnchor="end" fill="#829087" fontSize="10" fontFamily="Segoe UI, sans-serif">{guide}</text></g>)}
    {series.map((item) => <polyline key={item.label} points={points(item.values, x, y)} fill="none" stroke={item.color} strokeWidth="1.6" vectorEffect="non-scaling-stroke" />)}
    <text x="952" y={top + 17} textAnchor="end" fill="#a0aca4" fontSize="11" fontFamily="Segoe UI, Consolas, sans-serif">{series.map((item) => `${item.label} ${lastDefined(item.values)?.toFixed(1) ?? "-"}`).join(" · ")}</text>
  </g>;
}

export function InteractiveCandleChart({ snapshot, indicators }: { snapshot: ChartSnapshot; indicators: Set<string> }) {
  const width = 960;
  const priceTop = 28;
  const priceBottom = indicators.has("volume") ? 352 : 404;
  const sourceBars = snapshot.bars.slice(-Math.min(500, snapshot.bars.length));
  const [visibleCount, setVisibleCount] = useState(() => Math.min(120, sourceBars.length));
  const [endIndex, setEndIndex] = useState(sourceBars.length);
  const sliceStart = Math.max(0, endIndex - visibleCount);
  const bars = sourceBars.slice(sliceStart, endIndex);
  const calculationBars = sourceBars.slice(0, endIndex);
  const calculationCloses = calculationBars.map((bar) => bar.closeMinor);
  const priceValues = bars.flatMap((bar) => [bar.lowMinor, bar.highMinor]);
  const minimum = Math.min(...priceValues);
  const maximum = Math.max(...priceValues);
  const range = Math.max(1, maximum - minimum);
  const xStep = width / Math.max(1, bars.length);
  const x = (index: number) => xStep * index + xStep / 2;
  const y = (value: number) => priceBottom - ((value - minimum) / range) * (priceBottom - priceTop);
  const maxVolume = Math.max(1, ...bars.map((bar) => bar.volume));
  const panelIds = ["rsi", "macd", "stochastic", "cci", "dmi", "obv", "mfi", "momentum", "williams", "ultimateRsi", "volumeDelta"].filter((id) => indicators.has(id));
  const panelHeight = 112;
  const svgHeight = 430 + panelIds.length * panelHeight;
  const key = storageKey(snapshot);
  const svgRef = useRef<SVGSVGElement>(null);
  const [drawingMode, setDrawingMode] = useState(false);
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  const [drawings, setDrawings] = useState<Drawing[]>(() => readDrawings(key));
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hover, setHover] = useState<HoverPoint | null>(null);
  const [panOrigin, setPanOrigin] = useState<PanOrigin | null>(null);

  useEffect(() => {
    setDrawings(readDrawings(key)); setSelectedId(null); setAnchor(null); setHover(null);
    setVisibleCount(Math.min(120, sourceBars.length)); setEndIndex(sourceBars.length);
  }, [key, sourceBars.length]);
  const persist = (next: Drawing[]) => {
    setDrawings(next);
    try { localStorage.setItem(key, JSON.stringify(next)); }
    catch { /* 화면 동작은 유지하고 다음 실행의 복원만 생략한다. */ }
  };
  const pointerToValue = (event: PointerEvent<SVGSVGElement>) => {
    const rect = svgRef.current?.getBoundingClientRect();
    if (!rect || !bars.length) return null;
    const chartX = Math.max(0, Math.min(width, (event.clientX - rect.left) / rect.width * width));
    const chartY = Math.max(priceTop, Math.min(priceBottom, (event.clientY - rect.top) / rect.height * svgHeight));
    const index = Math.max(0, Math.min(bars.length - 1, Math.floor(chartX / xStep)));
    return { startTime: bars[index].periodStartMs, startPriceMinor: Math.round(maximum - ((chartY - priceTop) / (priceBottom - priceTop)) * range), index, chartX, chartY };
  };
  const handlePointerDown = (event: PointerEvent<SVGSVGElement>) => {
    if (event.button !== 0) return;
    if (!drawingMode) {
      event.currentTarget.setPointerCapture(event.pointerId);
      setPanOrigin({ clientX: event.clientX, endIndex });
      return;
    }
    const point = pointerToValue(event);
    if (!point) return;
    const drawingPoint = { startTime: point.startTime, startPriceMinor: point.startPriceMinor };
    if (!anchor) { setAnchor(drawingPoint); return; }
    const next = [...drawings, { id: crypto.randomUUID(), startTime: anchor.startTime, startPriceMinor: anchor.startPriceMinor, endTime: point.startTime, endPriceMinor: point.startPriceMinor }];
    persist(next); setAnchor(null); setDrawingMode(false); setSelectedId(next[next.length - 1]?.id ?? null);
  };
  const handlePointerMove = (event: PointerEvent<SVGSVGElement>) => {
    const point = pointerToValue(event);
    if (point) setHover({ index: point.index, priceMinor: point.startPriceMinor, chartX: point.chartX, chartY: point.chartY });
    if (!panOrigin || drawingMode || !bars.length) return;
    const rect = svgRef.current?.getBoundingClientRect();
    if (!rect) return;
    const shiftedBars = Math.round((panOrigin.clientX - event.clientX) / rect.width * visibleCount);
    const minimumEnd = Math.min(sourceBars.length, visibleCount);
    setEndIndex(Math.max(minimumEnd, Math.min(sourceBars.length, panOrigin.endIndex + shiftedBars)));
  };
  const finishPan = (event: PointerEvent<SVGSVGElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    setPanOrigin(null);
  };
  const changeZoom = (nextCount: number, focusRatio = .5) => {
    const bounded = Math.max(Math.min(20, sourceBars.length), Math.min(sourceBars.length, nextCount));
    if (!sourceBars.length || bounded === visibleCount) return;
    const focusIndex = sliceStart + Math.round((Math.max(0, Math.min(1, focusRatio))) * Math.max(0, visibleCount - 1));
    const nextEnd = Math.round(focusIndex + bounded * (1 - focusRatio));
    setVisibleCount(bounded);
    setEndIndex(Math.max(bounded, Math.min(sourceBars.length, nextEnd)));
  };
  const handleWheel = (event: WheelEvent<SVGSVGElement>) => {
    event.preventDefault();
    const rect = svgRef.current?.getBoundingClientRect();
    const focusRatio = rect ? (event.clientX - rect.left) / rect.width : .5;
    changeZoom(visibleCount + (event.deltaY > 0 ? 12 : -12), focusRatio);
  };
  const resetViewport = () => { setVisibleCount(Math.min(120, sourceBars.length)); setEndIndex(sourceBars.length); setHover(null); };
  const removeSelected = () => { if (!selectedId) return; persist(drawings.filter((item) => item.id !== selectedId)); setSelectedId(null); };
  const indexForTime = (time: number) => {
    if (!bars.length || time <= bars[0].periodStartMs) return 0;
    const index = bars.findIndex((bar) => bar.periodStartMs >= time);
    return index < 0 ? bars.length - 1 : index;
  };
  const sliceValues = (values: Array<number | null>) => values.slice(sliceStart, endIndex);

  const overlays = useMemo(() => {
    const result: Array<{ id: string; color: string; values: Array<number | null>; dash?: string }> = [];
    ([5, 20, 60, 120] as const).forEach((period) => { if (indicators.has(`ma${period}`)) result.push({ id: `MA ${period}`, color: ({ 5: "#e9cf68", 20: "#65b7d4", 60: "#d8869b", 120: "#a889dc" } as const)[period], values: sliceValues(sma(calculationCloses, period)) }); });
    ([20, 60] as const).forEach((period) => { if (indicators.has(`ema${period}`)) result.push({ id: `EMA ${period}`, color: period === 20 ? "#64d1a1" : "#d79858", values: sliceValues(ema(calculationCloses, period)), dash: "4 2" }); });
    if (indicators.has("bollinger")) { const data = bollinger(calculationCloses).slice(sliceStart, endIndex); result.push({ id: "BB 상단", color: "#7f9ee8", values: data.map((item) => item?.upper ?? null) }, { id: "BB 하단", color: "#7f9ee8", values: data.map((item) => item?.lower ?? null) }); }
    if (indicators.has("envelope")) { const middle = sliceValues(sma(calculationCloses, 20)); result.push({ id: "Envelope 상단", color: "#c18bd2", values: middle.map((value) => value == null ? null : value * 1.05), dash: "3 3" }, { id: "Envelope 하단", color: "#c18bd2", values: middle.map((value) => value == null ? null : value * .95), dash: "3 3" }); }
    if (indicators.has("ichimoku")) {
      const range9 = rollingRange(calculationBars, 9); const range26 = rollingRange(calculationBars, 26); const range52 = rollingRange(calculationBars, 52);
      const conversion = range9.map((item) => item ? (item.high + item.low) / 2 : null);
      const base = range26.map((item) => item ? (item.high + item.low) / 2 : null);
      const spanA = calculationBars.map((_, index) => index < 26 || conversion[index - 26] == null || base[index - 26] == null ? null : (conversion[index - 26]! + base[index - 26]!) / 2);
      const spanB = calculationBars.map((_, index) => index < 26 || range52[index - 26] == null ? null : (range52[index - 26]!.high + range52[index - 26]!.low) / 2);
      const lagging = calculationBars.map((_, index) => calculationCloses[index + 26] ?? null);
      result.push({ id: "전환선", color: "#e2b76d", values: sliceValues(conversion) }, { id: "기준선", color: "#7fc1d5", values: sliceValues(base) }, { id: "선행스팬 A", color: "#78b88e", values: sliceValues(spanA) }, { id: "선행스팬 B", color: "#c47b7b", values: sliceValues(spanB) }, { id: "후행스팬", color: "#9b82c8", values: sliceValues(lagging), dash: "3 3" });
    }
    return result;
  }, [calculationBars, calculationCloses, endIndex, indicators, sliceStart]);

  const panel = (id: string, top: number) => {
    if (id === "rsi") return <IndicatorPanel key={id} label="RSI (14)" bars={bars} top={top} height={panelHeight} guides={[30, 70]} series={[{ label: "RSI", color: "#b49ae8", values: sliceValues(rsi(calculationCloses)) }]} />;
    if (id === "macd") { const value = macd(calculationCloses); return <IndicatorPanel key={id} label="MACD (12, 26, 9)" bars={bars} top={top} height={panelHeight} guides={[0]} series={[{ label: "MACD", color: "#73b6da", values: sliceValues(value.line) }, { label: "SIGNAL", color: "#df9a65", values: sliceValues(value.signal) }]} />; }
    if (id === "stochastic") { const value = stochastic(calculationBars); return <IndicatorPanel key={id} label="Slow Stochastic (14, 3, 3)" bars={bars} top={top} height={panelHeight} guides={[20, 80]} series={[{ label: "%K", color: "#76c69e", values: sliceValues(value.k) }, { label: "%D", color: "#e6b86c", values: sliceValues(value.d) }]} />; }
    if (id === "cci") return <IndicatorPanel key={id} label="CCI (20)" bars={bars} top={top} height={panelHeight} guides={[-100, 0, 100]} series={[{ label: "CCI", color: "#d89bd3", values: sliceValues(cci(calculationBars)) }]} />;
    if (id === "dmi") { const value = dmi(calculationBars); return <IndicatorPanel key={id} label="DMI / ADX (14)" bars={bars} top={top} height={panelHeight} guides={[20]} series={[{ label: "+DI", color: "#71c69b", values: sliceValues(value.plus) }, { label: "-DI", color: "#e07979", values: sliceValues(value.minus) }, { label: "ADX", color: "#e1c56a", values: sliceValues(value.adx) }]} />; }
    if (id === "obv") return <IndicatorPanel key={id} label="OBV" bars={bars} top={top} height={panelHeight} guides={[0]} series={[{ label: "OBV", color: "#78b7d6", values: sliceValues(obv(calculationBars)) }]} />;
    if (id === "mfi") return <IndicatorPanel key={id} label="MFI (14)" bars={bars} top={top} height={panelHeight} guides={[20, 80]} series={[{ label: "MFI", color: "#c7a86d", values: sliceValues(mfi(calculationBars)) }]} />;
    if (id === "momentum") return <IndicatorPanel key={id} label="Momentum (10)" bars={bars} top={top} height={panelHeight} guides={[0]} series={[{ label: "MOM", color: "#70c9bb", values: sliceValues(momentum(calculationCloses)) }]} />;
    if (id === "williams") return <IndicatorPanel key={id} label="Williams %R (14)" bars={bars} top={top} height={panelHeight} guides={[-80, -20]} series={[{ label: "%R", color: "#d490a9", values: sliceValues(williamsR(calculationBars)) }]} />;
    if (id === "ultimateRsi") return <IndicatorPanel key={id} label="TELEGRAM · Filter RSI (14)" bars={bars} top={top} height={panelHeight} guides={[20, 50, 80]} series={[{ label: "URSI", color: "#d5e66d", values: sliceValues(ultimateRsi(calculationCloses)) }]} />;
    return <IndicatorPanel key={id} label="TELEGRAM · 캔들 거래량 델타 (추정)" bars={bars} top={top} height={panelHeight} guides={[0]} series={[{ label: "DELTA", color: "#7eb3d1", values: sliceValues(volumeDeltaEstimate(calculationBars)) }]} />;
  };

  return <div className={`paper-interactive-chart ${drawingMode ? "is-drawing" : ""}`}>
    <div className="paper-drawing-toolbar" aria-label="차트 그리기 도구">
      <button type="button" className={drawingMode ? "is-active" : ""} aria-pressed={drawingMode} onClick={() => { setDrawingMode((current) => !current); setAnchor(null); setSelectedId(null); }}>／ 추세선</button>
      <span>{drawingMode ? anchor ? "끝점을 선택하세요" : "시작점을 선택하세요" : `${drawings.length}개 자동 저장됨`}</span>
      <div className="paper-viewport-controls" aria-label="차트 보기 범위">
        <button type="button" aria-label="차트 확대" disabled={visibleCount <= Math.min(20, sourceBars.length)} onClick={() => changeZoom(visibleCount - 12)}>＋</button>
        <output aria-live="polite">{bars.length} / {sourceBars.length}봉</output>
        <button type="button" aria-label="차트 축소" disabled={visibleCount >= sourceBars.length} onClick={() => changeZoom(visibleCount + 12)}>－</button>
        <button type="button" onClick={resetViewport}>보기 초기화</button>
      </div>
      {selectedId && <button type="button" className="is-delete" onClick={removeSelected}>선 삭제</button>}
    </div>
    <div className="paper-chart-help">휠 확대·축소 · 빈 차트 드래그로 구간 이동 · 캔들에 마우스를 올려 시세 확인</div>
    <svg ref={svgRef} className={`paper-candle-chart ${panOrigin ? "is-panning" : ""}`} viewBox={`0 0 ${width} ${svgHeight}`} style={{ aspectRatio: `${width} / ${svgHeight}` }} onPointerDown={handlePointerDown} onPointerMove={handlePointerMove} onPointerUp={finishPan} onPointerCancel={finishPan} onPointerLeave={() => { if (!panOrigin) setHover(null); }} onWheel={handleWheel} role="img" aria-label={`${snapshot.symbol} ${snapshot.interval} 캔들 차트. 현재 ${bars.length}개 봉, 추세선 ${drawings.length}개.`}>
      <rect width={width} height={svgHeight} fill="#0a0f0c" />
      {[0, 1, 2, 3, 4].map((line) => <line key={line} x1="0" x2={width} y1={priceTop + ((priceBottom - priceTop) / 4) * line} y2={priceTop + ((priceBottom - priceTop) / 4) * line} stroke="#26332c" />)}
      {bars.map((bar, index) => { const up = bar.closeMinor >= bar.openMinor; const color = up ? "#e05f62" : "#5d8ed7"; const bodyTop = y(Math.max(bar.openMinor, bar.closeMinor)); return <g key={`${bar.periodStartMs}-${index}`} opacity={bar.completed ? 1 : .56}><line x1={x(index)} x2={x(index)} y1={y(bar.highMinor)} y2={y(bar.lowMinor)} stroke={color} strokeWidth="1.2" /><rect x={x(index) - Math.max(1.5, xStep * .3)} y={bodyTop} width={Math.max(3, xStep * .6)} height={Math.max(2, Math.abs(y(bar.openMinor) - y(bar.closeMinor)))} fill={color} />{indicators.has("volume") && <rect x={x(index) - Math.max(1, xStep * .26)} y={420 - bar.volume / maxVolume * 54} width={Math.max(2, xStep * .52)} height={bar.volume / maxVolume * 54} fill={color} opacity=".45" />}</g>; })}
      {overlays.map((item) => <polyline key={item.id} points={points(item.values, x, y)} fill="none" stroke={item.color} strokeWidth="1.7" strokeDasharray={item.dash} vectorEffect="non-scaling-stroke" />)}
      {indicators.has("sar") && parabolicSar(calculationBars).slice(sliceStart, endIndex).map((value, index) => <circle key={bars[index].periodStartMs} cx={x(index)} cy={y(value)} r="1.8" fill="#e5db87" />)}
      {indicators.has("bigSales") && bigSales(calculationBars).slice(sliceStart, endIndex).map((item, index) => item && <circle key={bars[index].periodStartMs} cx={x(index)} cy={y(item.price)} r={2 + item.strength * 1.2} fill={item.side === "buy" ? "#e8ece9" : "#5689d1"} opacity=".5" />)}
      {indicators.has("institutionalShift") && institutionalShift(calculationBars).slice(sliceStart, endIndex).map((item, index) => item && <g key={bars[index].periodStartMs}><circle cx={x(index)} cy={y(item.price)} r="6" fill={item.side === "buy" ? "#2b86dd" : "#e35151"} opacity=".32" /><circle cx={x(index)} cy={y(item.price)} r="2.5" fill={item.side === "buy" ? "#65b7f2" : "#ff7770"} /></g>)}
      {indicators.has("volumeProfile") && (() => { const buckets = 18; const totals = Array(buckets).fill(0) as number[]; bars.forEach((bar) => { const bucket = Math.min(buckets - 1, Math.max(0, Math.floor((bar.closeMinor - minimum) / range * buckets))); totals[bucket] += bar.volume; }); const peak = Math.max(1, ...totals); return totals.map((total, index) => <rect key={index} x={width - total / peak * 150} y={priceBottom - (index + 1) * (priceBottom - priceTop) / buckets} width={total / peak * 150} height={(priceBottom - priceTop) / buckets - 1} fill="#a5bd78" opacity=".18" />); })()}
      {drawings.filter((drawing) => bars.length && Math.max(drawing.startTime, drawing.endTime) >= bars[0].periodStartMs && Math.min(drawing.startTime, drawing.endTime) <= bars[bars.length - 1].periodStartMs).map((drawing) => { const selected = drawing.id === selectedId; const select = () => { setSelectedId(drawing.id); setDrawingMode(false); }; return <g key={drawing.id} tabIndex={0} role="button" aria-label={`저장된 추세선 ${formatMoney(drawing.startPriceMinor, snapshot.currency)}에서 ${formatMoney(drawing.endPriceMinor, snapshot.currency)}`} onPointerDown={(event) => { event.stopPropagation(); select(); }} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); select(); } }}><line x1={x(indexForTime(drawing.startTime))} y1={y(drawing.startPriceMinor)} x2={x(indexForTime(drawing.endTime))} y2={y(drawing.endPriceMinor)} stroke="transparent" strokeWidth="14" vectorEffect="non-scaling-stroke" /><line x1={x(indexForTime(drawing.startTime))} y1={y(drawing.startPriceMinor)} x2={x(indexForTime(drawing.endTime))} y2={y(drawing.endPriceMinor)} stroke={selected ? "#fff3a3" : "#d7ee65"} strokeWidth={selected ? 4 : 2.2} vectorEffect="non-scaling-stroke" /></g>; })}
      {anchor && <circle cx={x(indexForTime(anchor.startTime))} cy={y(anchor.startPriceMinor)} r="5" fill="#d7ee65" />}
      {hover && bars[hover.index] && <g className="paper-chart-crosshair" pointerEvents="none">
        <line x1={x(hover.index)} x2={x(hover.index)} y1={priceTop} y2={svgHeight} />
        <line x1="0" x2={width} y1={hover.chartY} y2={hover.chartY} />
        <rect x={hover.chartX > width - 310 ? hover.chartX - 304 : hover.chartX + 8} y={Math.max(priceTop + 5, Math.min(priceBottom - 88, hover.chartY + 8))} width="296" height="78" />
        <text x={hover.chartX > width - 310 ? hover.chartX - 294 : hover.chartX + 18} y={Math.max(priceTop + 22, Math.min(priceBottom - 71, hover.chartY + 25))}>
          <tspan>{new Intl.DateTimeFormat("ko-KR", snapshot.interval === "1m" ? { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" } : { year: "numeric", month: "2-digit", day: "2-digit" }).format(bars[hover.index].periodStartMs)}</tspan>
          <tspan x={hover.chartX > width - 310 ? hover.chartX - 294 : hover.chartX + 18} dy="17">시 {formatMoney(bars[hover.index].openMinor, snapshot.currency)} · 고 {formatMoney(bars[hover.index].highMinor, snapshot.currency)}</tspan>
          <tspan x={hover.chartX > width - 310 ? hover.chartX - 294 : hover.chartX + 18} dy="17">저 {formatMoney(bars[hover.index].lowMinor, snapshot.currency)} · 종 {formatMoney(bars[hover.index].closeMinor, snapshot.currency)}</tspan>
          <tspan x={hover.chartX > width - 310 ? hover.chartX - 294 : hover.chartX + 18} dy="17">거래량 {new Intl.NumberFormat("ko-KR", { notation: "compact", maximumFractionDigits: 2 }).format(bars[hover.index].volume)}</tspan>
        </text>
        <rect className="paper-chart-price-tag" x={width - 90} y={Math.max(priceTop, Math.min(priceBottom - 19, hover.chartY - 9))} width="90" height="18" />
        <text className="paper-chart-price-label" x={width - 5} y={Math.max(priceTop + 13, Math.min(priceBottom - 5, hover.chartY + 4))} textAnchor="end">{formatMoney(hover.priceMinor, snapshot.currency)}</text>
      </g>}
      <text x="12" y="18" fill="#89988f" fontSize="11" fontFamily="Consolas">{snapshot.provider} · {bars.length} BARS · {snapshot.adjusted ? "ADJUSTED" : "RAW"}</text>
      <text x={width - 12} y="18" textAnchor="end" fill="#dbe4dc" fontSize="12" fontFamily="Consolas">{formatMoney(bars[bars.length - 1]?.closeMinor ?? 0, snapshot.currency)}</text>
      {panelIds.map((id, index) => panel(id, 430 + index * panelHeight))}
    </svg>
  </div>;
}
