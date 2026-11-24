import * as echarts from 'echarts';
import type { EChartsCoreOption } from 'echarts/core';

import { formatDateNumeric, formatMoney, formatNumber } from '../format';
import type { DistributionBucket } from '../models';
import type { ReportData } from './report-data';
import { PDF_COLORS } from './report-theme';

const FONT = 'Helvetica, Arial, sans-serif';

/** Renderiza un gráfico ECharts fuera de pantalla y devuelve un PNG en alta resolución. */
export function renderChart(option: EChartsCoreOption, width: number, height: number): string {
  const host = document.createElement('div');
  host.style.cssText = `position:fixed;left:-20000px;top:0;width:${width}px;height:${height}px;`;
  document.body.appendChild(host);
  const chart = echarts.init(host, null, { renderer: 'canvas', width, height });
  try {
    chart.setOption({ animation: false, textStyle: { fontFamily: FONT }, ...option });
    return chart.getDataURL({ type: 'png', pixelRatio: 5, backgroundColor: '#ffffff' });
  } finally {
    chart.dispose();
    host.remove();
  }
}

function axisLabel(formatter?: (value: number) => string) {
  return {
    color: PDF_COLORS.muted,
    fontSize: 10,
    ...(formatter ? { formatter } : {}),
  };
}

const splitLine = { lineStyle: { color: PDF_COLORS.grid } };

/** Límites redondeados (múltiplos de un paso 1/2/2,5/5·10^n) que siempre incluyen el 0. */
function niceBounds(values: number[], ticks = 5): { min: number; max: number; interval: number } {
  let min = Math.min(0, ...values);
  let max = Math.max(0, ...values);
  if (min === max) max = min + 1;
  const raw = (max - min) / ticks;
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((factor) => factor * magnitude).find((value) => value >= raw) ?? raw;
  // Una caída mínima (< 10% de un paso) no justifica un tramo negativo entero.
  min = min > -step * 0.1 ? 0 : Math.floor(min / step) * step;
  max = Math.ceil(max / step) * step;
  return { min, max, interval: step };
}

/** Curva de retorno en % (eje izquierdo) y en dinero (eje derecho), con drawdown debajo. */
export function equityChart(data: ReportData): EChartsCoreOption {
  const points = [
    { date: data.periodFrom ?? data.equity[0]?.date ?? '', pct: 0, drawdownPct: 0 },
    ...data.equity,
  ];
  const labels = points.map((point) => point.date);
  const pctValues = points.map((point) => Number(point.pct.toFixed(4)));
  const { min, max, interval } = niceBounds(pctValues);
  const ddValues = points.map((point) => Number(point.drawdownPct.toFixed(4)));
  const dd = niceBounds([Math.min(-1, ...ddValues)], 2);
  const showMoney = !data.options.hideSensitive;
  const base = data.startBalance;
  const final = pctValues[pctValues.length - 1] ?? 0;
  const lineColor = final >= 0 ? PDF_COLORS.accent : PDF_COLORS.neg;

  return {
    grid: [
      { left: 58, right: showMoney ? 92 : 24, top: 18, height: '58%' },
      { left: 58, right: showMoney ? 92 : 24, top: '74%', bottom: 34 },
    ],
    xAxis: [
      {
        type: 'category',
        data: labels,
        gridIndex: 0,
        boundaryGap: false,
        axisLabel: { show: false },
        axisTick: { show: false },
        axisLine: { lineStyle: { color: PDF_COLORS.border } },
      },
      {
        type: 'category',
        data: labels,
        gridIndex: 1,
        boundaryGap: false,
        axisTick: { show: false },
        axisLine: { lineStyle: { color: PDF_COLORS.border } },
        axisLabel: {
          ...axisLabel(),
          hideOverlap: true,
          formatter: (value: string) => formatDateNumeric(value),
        },
      },
    ],
    yAxis: [
      {
        type: 'value',
        gridIndex: 0,
        min,
        max,
        interval,
        axisLabel: axisLabel((value) => `${formatNumber(value, interval < 1 ? 1 : 0)}%`),
        splitLine,
      },
      ...(showMoney
        ? [
            {
              type: 'value',
              gridIndex: 0,
              position: 'right',
              min,
              max,
              interval,
              splitLine: { show: false },
              axisLabel: axisLabel((value: number) =>
                formatMoney((value / 100) * base, data.currency).replace(/,\d\d(?=\D*$)/, ''),
              ),
            },
          ]
        : []),
      {
        type: 'value',
        gridIndex: 1,
        min: dd.min,
        max: 0,
        interval: dd.interval,
        axisLabel: axisLabel((value) => `${formatNumber(value, dd.interval < 1 ? 1 : 0)}%`),
        splitLine,
      },
    ],
    series: [
      {
        type: 'line',
        xAxisIndex: 0,
        yAxisIndex: 0,
        data: pctValues,
        showSymbol: false,
        smooth: 0.15,
        clip: false,
        lineStyle: { width: 1.8, color: lineColor },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: final >= 0 ? 'rgba(79, 91, 213, 0.22)' : 'rgba(217, 67, 78, 0.18)' },
            { offset: 1, color: 'rgba(255, 255, 255, 0)' },
          ]),
        },
        markLine: {
          silent: true,
          symbol: 'none',
          label: { show: false },
          lineStyle: { color: PDF_COLORS.muted, type: 'dashed', width: 1 },
          data: [{ yAxis: 0 }],
        },
      },
      {
        type: 'line',
        xAxisIndex: 1,
        yAxisIndex: showMoney ? 2 : 1,
        data: ddValues,
        showSymbol: false,
        lineStyle: { width: 1.2, color: PDF_COLORS.neg },
        areaStyle: { color: 'rgba(217, 67, 78, 0.16)' },
      },
    ],
  };
}

/** P&L diario (dinero o % si se ocultan importes). */
export function dailyChart(data: ReportData): EChartsCoreOption {
  const hide = data.options.hideSensitive;
  const values = data.days.map((day) => Number((hide ? day.pct : day.net).toFixed(4)));
  return {
    grid: { left: hide ? 52 : 72, right: 16, top: 14, bottom: 34 },
    xAxis: {
      type: 'category',
      data: data.days.map((day) => day.key),
      axisTick: { show: false },
      axisLine: { lineStyle: { color: PDF_COLORS.border } },
      axisLabel: { ...axisLabel(), hideOverlap: true, formatter: formatDateNumeric },
    },
    yAxis: {
      type: 'value',
      splitNumber: 4,
      splitLine,
      axisLabel: axisLabel((value) =>
        hide
          ? `${formatNumber(value, 1)}%`
          : formatMoney(value, data.currency).replace(/,\d\d(?=\D*$)/, ''),
      ),
    },
    series: [
      {
        type: 'bar',
        barMaxWidth: 14,
        data: values.map((value) => ({
          value,
          itemStyle: {
            color: value >= 0 ? PDF_COLORS.pos : PDF_COLORS.neg,
            borderRadius: value >= 0 ? [2, 2, 0, 0] : [0, 0, 2, 2],
          },
        })),
      },
    ],
  };
}

export function rDistributionChart(buckets: DistributionBucket[]): EChartsCoreOption {
  return {
    grid: { left: 40, right: 12, top: 22, bottom: 30 },
    xAxis: {
      type: 'category',
      data: buckets.map((bucket) => bucket.label),
      axisTick: { show: false },
      axisLine: { lineStyle: { color: PDF_COLORS.border } },
      axisLabel: axisLabel(),
    },
    yAxis: { type: 'value', minInterval: 1, splitNumber: 4, splitLine, axisLabel: axisLabel() },
    series: [
      {
        type: 'bar',
        barMaxWidth: 34,
        label: { show: true, position: 'top', color: PDF_COLORS.text, fontSize: 10 },
        data: buckets.map((bucket, index) => ({
          value: bucket.count,
          itemStyle: {
            color: bucket.label.startsWith("<") || bucket.label.startsWith("-") ? PDF_COLORS.neg : PDF_COLORS.pos,
            borderRadius: [3, 3, 0, 0],
          },
        })),
      },
    ],
  };
}
