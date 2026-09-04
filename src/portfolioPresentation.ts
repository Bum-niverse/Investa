import type { MeetingBuyingPowerEvidence, MeetingPositionEvidence } from "./meetingEvidence";

export type PortfolioSlice = {
  id: string;
  symbol: string;
  name: string;
  marketValue: number;
  costBasis: number;
  profitLoss: number;
  quantity: number;
  weightBps: number;
  color: string;
  isOther: boolean;
};

export type PortfolioCurrencyGroup = {
  currency: string;
  marketValue: number;
  costBasis: number;
  profitLoss: number;
  buyingPower: number | null;
  slices: PortfolioSlice[];
  gradient: string;
};

const COLORS = ["#20b7a8", "#2779a8", "#3154a4", "#c94660", "#ef9f2f", "#68c6cb", "#7ddc91"];

const parseDecimal = (value: string) => {
  if (!/^-?\d+(?:\.\d+)?$/.test(value.trim())) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};

const normalizedCurrency = (position: MeetingPositionEvidence) =>
  position.currency.trim().toUpperCase() || (position.marketCountry.toUpperCase() === "US" ? "USD" : "KRW");

export function buildPortfolioCurrencyGroups(
  positions: MeetingPositionEvidence[],
  buyingPower: MeetingBuyingPowerEvidence[] = [],
): PortfolioCurrencyGroup[] {
  const holdingMap = new Map<string, { currency: string; symbol: string; name: string; quantity: number; marketValue: number; costBasis: number }>();
  for (const position of positions) {
    const quantity = parseDecimal(position.quantity);
    const lastPrice = parseDecimal(position.lastPrice);
    const averagePrice = parseDecimal(position.averagePurchasePrice);
    if (quantity == null || lastPrice == null || averagePrice == null || quantity < 0 || lastPrice < 0 || averagePrice < 0) continue;
    const currency = normalizedCurrency(position);
    const key = `${currency}:${position.symbol.trim().toUpperCase()}`;
    const current = holdingMap.get(key) ?? { currency, symbol: position.symbol, name: position.name, quantity: 0, marketValue: 0, costBasis: 0 };
    current.quantity += quantity;
    current.marketValue += quantity * lastPrice;
    current.costBasis += quantity * averagePrice;
    holdingMap.set(key, current);
  }

  const buyingPowerByCurrency = new Map<string, number>();
  for (const item of buyingPower) {
    const value = parseDecimal(item.cashBuyingPower);
    if (value == null || value < 0) continue;
    const currency = item.currency.trim().toUpperCase();
    buyingPowerByCurrency.set(currency, (buyingPowerByCurrency.get(currency) ?? 0) + value);
  }

  const positiveBuyingPowerCurrencies = Array.from(buyingPowerByCurrency.entries())
    .filter(([, amount]) => amount > 0)
    .map(([currency]) => currency);
  const currencyKeys = new Set([...Array.from(holdingMap.values(), (item) => item.currency), ...positiveBuyingPowerCurrencies]);
  return Array.from(currencyKeys).sort().map((currency) => {
    const holdings = Array.from(holdingMap.values()).filter((item) => item.currency === currency).sort((a, b) => b.marketValue - a.marketValue);
    const marketValue = holdings.reduce((sum, item) => sum + item.marketValue, 0);
    const costBasis = holdings.reduce((sum, item) => sum + item.costBasis, 0);
    const visible = holdings.length > 6
      ? [...holdings.slice(0, 5), holdings.slice(5).reduce((other, item) => ({
          currency,
          symbol: "OTHER",
          name: `기타 ${holdings.length - 5}종목`,
          quantity: 0,
          marketValue: other.marketValue + item.marketValue,
          costBasis: other.costBasis + item.costBasis,
        }), { currency, symbol: "OTHER", name: "기타", quantity: 0, marketValue: 0, costBasis: 0 })]
      : holdings;
    let cursor = 0;
    const slices = visible.map((item, index) => {
      const weightBps = marketValue > 0 ? Math.round((item.marketValue / marketValue) * 10_000) : 0;
      const slice = {
        id: `${currency}:${item.symbol}`,
        symbol: item.symbol,
        name: item.name,
        marketValue: item.marketValue,
        costBasis: item.costBasis,
        profitLoss: item.marketValue - item.costBasis,
        quantity: item.quantity,
        weightBps,
        color: COLORS[index % COLORS.length],
        isOther: item.symbol === "OTHER",
      };
      cursor += weightBps;
      return slice;
    });
    let gradientCursor = 0;
    const gradientStops = slices.filter((slice) => slice.weightBps > 0).map((slice) => {
      const start = gradientCursor;
      gradientCursor += slice.weightBps / 100;
      return `${slice.color} ${start}% ${gradientCursor}%`;
    });
    return {
      currency,
      marketValue,
      costBasis,
      profitLoss: marketValue - costBasis,
      buyingPower: buyingPowerByCurrency.get(currency) ?? null,
      slices,
      gradient: gradientStops.length ? `conic-gradient(${gradientStops.join(", ")})` : "#253029",
    };
  });
}

export function formatPortfolioMoney(value: number, currency: string) {
  try {
    return new Intl.NumberFormat("ko-KR", {
      style: "currency",
      currency,
      maximumFractionDigits: currency === "KRW" ? 0 : 2,
    }).format(value);
  } catch {
    return `${currency} ${value.toLocaleString("ko-KR", { maximumFractionDigits: 2 })}`;
  }
}
