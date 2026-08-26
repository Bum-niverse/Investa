export type IndicatorBar = {
  periodStartMs: number;
  openMinor: number;
  highMinor: number;
  lowMinor: number;
  closeMinor: number;
  volume: number;
};

export type IndicatorDefinition = {
  id: string;
  label: string;
  group: "price" | "oscillator" | "volume" | "telegram";
  description: string;
  dataNote?: string;
};

export const INDICATOR_DEFINITIONS: IndicatorDefinition[] = [
  { id: "ma5", label: "MA 5", group: "price", description: "5봉 단순이동평균" },
  { id: "ma20", label: "MA 20", group: "price", description: "20봉 단순이동평균" },
  { id: "ma60", label: "MA 60", group: "price", description: "60봉 단순이동평균" },
  { id: "ma120", label: "MA 120", group: "price", description: "120봉 단순이동평균" },
  { id: "ema20", label: "EMA 20", group: "price", description: "20봉 지수이동평균" },
  { id: "ema60", label: "EMA 60", group: "price", description: "60봉 지수이동평균" },
  { id: "bollinger", label: "볼린저밴드", group: "price", description: "20봉 평균 ± 표준편차 2배" },
  { id: "envelope", label: "엔벨로프", group: "price", description: "20봉 평균 ± 5%" },
  { id: "ichimoku", label: "일목균형표", group: "price", description: "전환선·기준선·선행스팬" },
  { id: "sar", label: "Parabolic SAR", group: "price", description: "추세 추적형 반전점" },
  { id: "volumeProfile", label: "매물대 / VP BOX", group: "price", description: "표시 구간 18개 가격대별 거래량", dataNote: "텔레그램 VP BOX의 핵심 가격대 거래량 분포를 현재 표시 구간에 적용합니다." },
  { id: "volume", label: "거래량", group: "volume", description: "봉별 거래량" },
  { id: "rsi", label: "RSI", group: "oscillator", description: "14봉 상대강도지수" },
  { id: "macd", label: "MACD", group: "oscillator", description: "EMA 12·26과 9 시그널" },
  { id: "stochastic", label: "스토캐스틱", group: "oscillator", description: "14·3·3 Slow Stochastic" },
  { id: "cci", label: "CCI", group: "oscillator", description: "20봉 상품채널지수" },
  { id: "dmi", label: "DMI / ADX", group: "oscillator", description: "14봉 방향성과 추세 강도" },
  { id: "obv", label: "OBV", group: "volume", description: "가격 방향 누적 거래량" },
  { id: "mfi", label: "MFI", group: "volume", description: "14봉 가격·거래량 자금흐름" },
  { id: "momentum", label: "Momentum", group: "oscillator", description: "10봉 가격 변화" },
  { id: "williams", label: "Williams %R", group: "oscillator", description: "14봉 고저 범위 내 종가 위치" },
  { id: "ultimateRsi", label: "필터 RSI", group: "telegram", description: "텔레그램 Ultimate RSI 수식의 14봉 변형" },
  { id: "volumeDelta", label: "캔들 거래량 델타", group: "telegram", description: "종가의 고저 범위 위치로 매수·매도 거래량 추정", dataNote: "매수·매도 체결 원자료가 없어 Pine 원본의 OHLCV 근사식을 사용합니다." },
  { id: "institutionalShift", label: "Institutional Shift", group: "telegram", description: "20봉 평균 2배 거래량과 50% 이상 몸통 감지", dataNote: "기관 거래를 직접 확인하는 값이 아니라 원본 Pine의 변동 감지 조건입니다." },
  { id: "bigSales", label: "Big Sales", group: "telegram", description: "7봉 평균 대비 대량 거래 표시", dataNote: "텔레그램 Pine 수식을 OHLCV 범위에서 재현합니다." },
];

export const sma = (values: number[], period: number) => values.map((_, index) => {
  if (index + 1 < period) return null;
  let sum = 0;
  for (let cursor = index + 1 - period; cursor <= index; cursor += 1) sum += values[cursor];
  return sum / period;
});

export const ema = (values: number[], period: number) => {
  const output: Array<number | null> = Array(values.length).fill(null);
  if (values.length < period) return output;
  const multiplier = 2 / (period + 1);
  let previous = values.slice(0, period).reduce((sum, value) => sum + value, 0) / period;
  output[period - 1] = previous;
  for (let index = period; index < values.length; index += 1) {
    previous = (values[index] - previous) * multiplier + previous;
    output[index] = previous;
  }
  return output;
};

export const rsi = (values: number[], period = 14) => {
  const output: Array<number | null> = Array(values.length).fill(null);
  if (values.length <= period) return output;
  let gain = 0;
  let loss = 0;
  for (let index = 1; index <= period; index += 1) {
    const change = values[index] - values[index - 1];
    gain += Math.max(change, 0);
    loss += Math.max(-change, 0);
  }
  let averageGain = gain / period;
  let averageLoss = loss / period;
  output[period] = averageLoss === 0 ? 100 : 100 - 100 / (1 + averageGain / averageLoss);
  for (let index = period + 1; index < values.length; index += 1) {
    const change = values[index] - values[index - 1];
    averageGain = (averageGain * (period - 1) + Math.max(change, 0)) / period;
    averageLoss = (averageLoss * (period - 1) + Math.max(-change, 0)) / period;
    output[index] = averageLoss === 0 ? 100 : 100 - 100 / (1 + averageGain / averageLoss);
  }
  return output;
};

export const rollingRange = (bars: IndicatorBar[], period: number) => bars.map((_, index) => {
  if (index + 1 < period) return null;
  const window = bars.slice(index + 1 - period, index + 1);
  return { high: Math.max(...window.map((bar) => bar.highMinor)), low: Math.min(...window.map((bar) => bar.lowMinor)) };
});

export const bollinger = (values: number[], period = 20, multiplier = 2) => {
  const middle = sma(values, period);
  return middle.map((mean, index) => {
    if (mean == null) return null;
    const window = values.slice(index + 1 - period, index + 1);
    const deviation = Math.sqrt(window.reduce((sum, value) => sum + (value - mean) ** 2, 0) / period);
    return { middle: mean, upper: mean + deviation * multiplier, lower: mean - deviation * multiplier };
  });
};

export const macd = (values: number[]) => {
  const fast = ema(values, 12);
  const slow = ema(values, 26);
  const line = values.map((_, index) => fast[index] == null || slow[index] == null ? null : fast[index]! - slow[index]!);
  const compact = line.filter((value): value is number => value != null);
  const compactSignal = ema(compact, 9);
  let signalIndex = 0;
  const signal = line.map((value) => value == null ? null : compactSignal[signalIndex++]);
  return { line, signal, histogram: line.map((value, index) => value == null || signal[index] == null ? null : value - signal[index]!) };
};

export const stochastic = (bars: IndicatorBar[], period = 14) => {
  const ranges = rollingRange(bars, period);
  const fast = bars.map((bar, index) => {
    const range = ranges[index];
    return !range || range.high === range.low ? null : ((bar.closeMinor - range.low) / (range.high - range.low)) * 100;
  });
  const slowK = nullableSma(fast, 3);
  return { k: slowK, d: nullableSma(slowK, 3) };
};

export const nullableSma = (values: Array<number | null>, period: number) => values.map((_, index) => {
  const window = values.slice(index + 1 - period, index + 1);
  return window.length < period || window.some((value) => value == null) ? null : window.reduce<number>((sum, value) => sum + value!, 0) / period;
});

export const cci = (bars: IndicatorBar[], period = 20) => {
  const typical = bars.map((bar) => (bar.highMinor + bar.lowMinor + bar.closeMinor) / 3);
  const average = sma(typical, period);
  return typical.map((value, index) => {
    const mean = average[index];
    if (mean == null) return null;
    const deviation = typical.slice(index + 1 - period, index + 1).reduce((sum, item) => sum + Math.abs(item - mean), 0) / period;
    return deviation === 0 ? 0 : (value - mean) / (0.015 * deviation);
  });
};

export const obv = (bars: IndicatorBar[]) => {
  let total = 0;
  return bars.map((bar, index) => {
    if (index) total += bar.closeMinor > bars[index - 1].closeMinor ? bar.volume : bar.closeMinor < bars[index - 1].closeMinor ? -bar.volume : 0;
    return total;
  });
};

export const momentum = (values: number[], period = 10) => values.map((value, index) => index < period ? null : value - values[index - period]);

export const williamsR = (bars: IndicatorBar[], period = 14) => {
  const ranges = rollingRange(bars, period);
  return bars.map((bar, index) => {
    const range = ranges[index];
    return !range || range.high === range.low ? null : ((range.high - bar.closeMinor) / (range.high - range.low)) * -100;
  });
};

export const mfi = (bars: IndicatorBar[], period = 14) => {
  const typical = bars.map((bar) => (bar.highMinor + bar.lowMinor + bar.closeMinor) / 3);
  const flow = bars.map((bar, index) => typical[index] * bar.volume);
  return bars.map((_, index) => {
    if (index < period) return null;
    let positive = 0;
    let negative = 0;
    for (let cursor = index + 1 - period; cursor <= index; cursor += 1) {
      if (typical[cursor] >= typical[cursor - 1]) positive += flow[cursor]; else negative += flow[cursor];
    }
    return negative === 0 ? 100 : 100 - 100 / (1 + positive / negative);
  });
};

export const ultimateRsi = (values: number[], period = 14) => {
  const ranges = values.map((_, index) => {
    if (index + 1 < period) return null;
    const window = values.slice(index + 1 - period, index + 1);
    return { high: Math.max(...window), low: Math.min(...window) };
  });
  const differences = values.map((value, index) => {
    if (!index || !ranges[index]) return 0;
    const current = ranges[index]!;
    const previous = ranges[index - 1];
    const range = current.high - current.low;
    return !previous ? value - values[index - 1] : current.high > previous.high ? range : current.low < previous.low ? -range : value - values[index - 1];
  });
  const numerator = ema(differences, period);
  const denominator = ema(differences.map(Math.abs), period);
  return values.map((_, index) => numerator[index] == null || denominator[index] == null ? null : denominator[index] === 0 ? 50 : numerator[index]! / denominator[index]! * 50 + 50);
};

export const volumeDeltaEstimate = (bars: IndicatorBar[]) => bars.map((bar) => {
  const range = bar.highMinor - bar.lowMinor;
  if (range <= 0) return 0;
  const buyVolume = bar.volume * (bar.closeMinor - bar.lowMinor) / range;
  return buyVolume - (bar.volume - buyVolume);
});

export const institutionalShift = (bars: IndicatorBar[], period = 20) => {
  const averages = sma(bars.map((bar) => bar.volume), period);
  return bars.map((bar, index) => {
    const average = averages[index];
    const range = bar.highMinor - bar.lowMinor;
    if (average == null || range <= 0 || bar.volume < average * 2 || Math.abs(bar.closeMinor - bar.openMinor) / range < .5) return null;
    return { price: bar.closeMinor >= bar.openMinor ? bar.lowMinor : bar.highMinor, side: bar.closeMinor >= bar.openMinor ? "buy" as const : "sell" as const };
  });
};

export const bigSales = (bars: IndicatorBar[], period = 7) => {
  const volumeAverage = sma(bars.map((bar) => bar.volume), period);
  return bars.map((bar, index) => {
    const average = volumeAverage[index];
    if (average == null || bar.volume < average) return null;
    const ratio = bar.volume / average;
    return { price: bar.closeMinor, strength: ratio >= 2 ? 5 : ratio >= 1.75 ? 4 : ratio >= 1.5 ? 3 : ratio >= 1.25 ? 2 : 1, side: index && bar.closeMinor < bars[index - 1].closeMinor ? "sell" as const : "buy" as const };
  });
};

export const dmi = (bars: IndicatorBar[], period = 14) => {
  const trueRange = bars.map((bar, index) => index === 0 ? bar.highMinor - bar.lowMinor : Math.max(bar.highMinor - bar.lowMinor, Math.abs(bar.highMinor - bars[index - 1].closeMinor), Math.abs(bar.lowMinor - bars[index - 1].closeMinor)));
  const plus = bars.map((bar, index) => !index ? 0 : Math.max(bar.highMinor - bars[index - 1].highMinor, 0) > Math.max(bars[index - 1].lowMinor - bar.lowMinor, 0) ? Math.max(bar.highMinor - bars[index - 1].highMinor, 0) : 0);
  const minus = bars.map((bar, index) => !index ? 0 : Math.max(bars[index - 1].lowMinor - bar.lowMinor, 0) > Math.max(bar.highMinor - bars[index - 1].highMinor, 0) ? Math.max(bars[index - 1].lowMinor - bar.lowMinor, 0) : 0);
  const trAverage = sma(trueRange, period);
  const plusAverage = sma(plus, period);
  const minusAverage = sma(minus, period);
  const plusDi = bars.map((_, index) => !trAverage[index] ? null : plusAverage[index]! / trAverage[index]! * 100);
  const minusDi = bars.map((_, index) => !trAverage[index] ? null : minusAverage[index]! / trAverage[index]! * 100);
  const dx = bars.map((_, index) => plusDi[index] == null || minusDi[index] == null || plusDi[index]! + minusDi[index]! === 0 ? null : Math.abs(plusDi[index]! - minusDi[index]!) / (plusDi[index]! + minusDi[index]!) * 100);
  return { plus: plusDi, minus: minusDi, adx: nullableSma(dx, period) };
};

export const parabolicSar = (bars: IndicatorBar[]) => {
  if (!bars.length) return [];
  const output = [bars[0].lowMinor];
  let rising = true;
  let extreme = bars[0].highMinor;
  let acceleration = 0.02;
  for (let index = 1; index < bars.length; index += 1) {
    let next = output[index - 1] + acceleration * (extreme - output[index - 1]);
    if (rising) {
      next = Math.min(next, bars[index - 1].lowMinor, index > 1 ? bars[index - 2].lowMinor : bars[index - 1].lowMinor);
      if (bars[index].lowMinor < next) { rising = false; next = extreme; extreme = bars[index].lowMinor; acceleration = 0.02; }
      else if (bars[index].highMinor > extreme) { extreme = bars[index].highMinor; acceleration = Math.min(0.2, acceleration + 0.02); }
    } else {
      next = Math.max(next, bars[index - 1].highMinor, index > 1 ? bars[index - 2].highMinor : bars[index - 1].highMinor);
      if (bars[index].highMinor > next) { rising = true; next = extreme; extreme = bars[index].highMinor; acceleration = 0.02; }
      else if (bars[index].lowMinor < extreme) { extreme = bars[index].lowMinor; acceleration = Math.min(0.2, acceleration + 0.02); }
    }
    output.push(next);
  }
  return output;
};
