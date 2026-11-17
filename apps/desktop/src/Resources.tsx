import { useEffect, useRef, useState } from "react";
import { call, explain } from "./api";
import { Icon, type IconName } from "./Icon";
import { bytes } from "./Transfers";
import { createReadPoller } from "./read-poller";

type Sample = {
  cpu_percent: number | null;
  cores: number;
  memory_total: number;
  memory_available: number;
  swap_total: number;
  swap_free: number;
  disk_total: number;
  disk_used: number;
  disk_available: number;
  uptime_s: number;
  load: number[];
};
const percent = (part: number, total: number) =>
  total > 0 ? Math.max(0, Math.min(100, (part / total) * 100)) : null;
export function Resources({ space }: { space: string }) {
  const [sample, setSample] = useState<Sample>();
  const [series, setSeries] = useState<
    { cpu: number | null; memory: number | null }[]
  >([]);
  const [dimension, setDimension] = useState<"cpu" | "memory">("cpu");
  const [at, setAt] = useState<Date>();
  const [error, setError] = useState("");
  const poller = useRef<ReturnType<typeof createReadPoller<Sample>> | null>(null);
  const [sampling, setSampling] = useState(false);
  useEffect(() => {
    const loop = createReadPoller({
      read: () => call<Sample>("resources", { space }),
      onValue: (result) => {
        setSample(result);
        setAt(new Date());
        setError("");
        setSeries((old) => [
          ...old.slice(-39),
          {
            cpu: result.cpu_percent,
            memory: percent(
              result.memory_total - result.memory_available,
              result.memory_total,
            ),
          },
        ]);
      },
      onError: (e) => setError(explain(e)),
      onBusy: setSampling,
      visible: () => !document.hidden,
    });
    poller.current = loop;
    void loop.refresh();
    document.addEventListener("visibilitychange", loop.visibilityChanged);
    return () => {
      loop.stop();
      poller.current = null;
      document.removeEventListener("visibilitychange", loop.visibilityChanged);
    };
  }, [space]);
  const memory = sample ? sample.memory_total - sample.memory_available : 0;
  return (
    <section className="resources-page">
      <header className="resource-heading">
        <div>
          <h2>资源监控</h2>
          <p>
            {at
              ? `上次采样 ${at.toLocaleTimeString("zh-CN")}`
              : "正在读取服务器状态"}{" "}
            · Linux
          </p>
        </div>
        <button disabled={sampling} onClick={() => void poller.current?.refresh()}>
          <Icon name="refresh" size={15} />
          {sampling ? "采样中…" : "刷新"}
        </button>
      </header>
      {error && (
        <div className="error-banner" role="alert">
          <span>
            {error}
            {sample && " 以下保留上次采样。"}
          </span>
        </div>
      )}
      <div className="resource-cards">
        <Metric
          icon="activity"
          label="CPU 使用率"
          value={sample?.cpu_percent ?? null}
          detail={sample ? `${sample.cores} 个逻辑核心` : "等待采样"}
        />
        <Metric
          icon="server"
          label="内存使用率"
          value={sample ? percent(memory, sample.memory_total) : null}
          detail={
            sample
              ? `${bytes(memory)} / ${bytes(sample.memory_total)}`
              : "等待采样"
          }
        />
        <Metric
          icon="folder"
          label="存储使用率"
          value={sample ? percent(sample.disk_used, sample.disk_total) : null}
          detail={
            sample
              ? `${bytes(sample.disk_used)} / ${bytes(sample.disk_total)} · 根分区 /`
              : "等待采样"
          }
        />
      </div>
      <div className="resource-board">
        <section className="system-state" aria-labelledby="system-state-title">
          <div className="system-state-heading">
            <h3 id="system-state-title">当前资源使用</h3>
            <Icon name="activity" size={18} />
          </div>
          <div className="resource-focus-value">
            <strong>
              {(dimension === "cpu"
                ? sample?.cpu_percent
                : sample
                  ? percent(memory, sample.memory_total)
                  : null
              )?.toFixed(1) ?? "—"}
            </strong>
            <span>%</span>
          </div>
          <p>
            {dimension === "cpu" ? "CPU 使用率" : "内存使用率"} ·{" "}
            {sample ? "最近一次采样" : "等待服务器采样"}
          </p>
          <div className="system-facts">
            <div>
              <span>逻辑核心</span>
              <strong>{sample ? `${sample.cores} 核` : "—"}</strong>
            </div>
            <div>
              <span>系统运行时间</span>
              <strong>
                {sample
                  ? `${Math.floor(sample.uptime_s / 86400)} 天 ${Math.floor(sample.uptime_s / 3600) % 24} 小时`
                  : "—"}
              </strong>
            </div>
          </div>
          <div className="system-state-note">
            <Icon name="shield" size={14} />
            只读采样 · 无需安装远端服务
          </div>
        </section>
        <section className="cpu-history">
          <div className="trend-heading">
            <div>
              <h3>资源使用趋势</h3>
              <span>当前页面 · 最近 {series.length} 次采样</span>
            </div>
            <div className="trend-tabs" role="group" aria-label="趋势指标">
              <button
                aria-pressed={dimension === "cpu"}
                onClick={() => setDimension("cpu")}
              >
                CPU
              </button>
              <button
                aria-pressed={dimension === "memory"}
                onClick={() => setDimension("memory")}
              >
                内存
              </button>
            </div>
          </div>
          <div className="trend-current">
            <span>{dimension === "cpu" ? "CPU 使用率" : "内存使用率"}</span>
            <strong>
              {series.at(-1)?.[dimension]?.toFixed(1) ?? "—"}
              <small>%</small>
            </strong>
          </div>
          <div className="resource-chart">
            <span className="chart-max">100%</span>
            <span className="chart-mid">50%</span>
            <span className="chart-min">0%</span>
            {series.filter((s) => s[dimension] !== null).length > 1 ? (
              <svg
                viewBox="0 0 600 140"
                preserveAspectRatio="none"
                role="img"
                aria-label={`最近 ${series.length} 次${dimension === "cpu" ? "CPU" : "内存"}采样，最新 ${series.at(-1)?.[dimension]?.toFixed(1) ?? "未知"}%`}
              >
                <path
                  d={series
                    .map((s, i) =>
                      s[dimension] === null
                        ? ""
                        : `${i === 0 || series[i - 1][dimension] === null ? "M" : "L"} ${(i / (series.length - 1)) * 600},${140 - Math.max(0, Math.min(100, s[dimension]!)) * 1.4}`,
                    )
                    .join(" ")}
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  vectorEffect="non-scaling-stroke"
                />
              </svg>
            ) : (
              <p>
                {sample
                  ? "下一次采样后显示趋势"
                  : error
                    ? "采样不可用"
                    : "等待第一组采样…"}
              </p>
            )}
          </div>
          <div className="trend-caption">
            <span>较早采样</span>
            <span>最新采样</span>
          </div>
        </section>
      </div>
      <dl className="resource-details">
        <div>
          <dt>系统负载 · 1 / 5 / 15 分钟</dt>
          <dd>{sample?.load.map((n) => n.toFixed(2)).join(" / ") ?? "—"}</dd>
        </div>
        <div>
          <dt>可用内存</dt>
          <dd>{bytes(sample?.memory_available)}</dd>
        </div>
        <div>
          <dt>交换空间已用 / 总量</dt>
          <dd>
            {sample
              ? `${bytes(sample.swap_total - sample.swap_free)} / ${bytes(sample.swap_total)}`
              : "—"}
          </dd>
        </div>
        <div>
          <dt>根分区可用</dt>
          <dd>{bytes(sample?.disk_available)}</dd>
        </div>
        <div>
          <dt>运行时间</dt>
          <dd>
            {sample
              ? `${Math.floor(sample.uptime_s / 86400)} 天 ${Math.floor(sample.uptime_s / 3600) % 24} 小时 ${Math.floor(sample.uptime_s / 60) % 60} 分钟`
              : "—"}
          </dd>
        </div>
      </dl>
      <p className="resource-note">
        <Icon name="clock" size={14} />
        每轮约 1 秒采样，间隔 5 秒刷新；离开此页暂停后续采样。
      </p>
    </section>
  );
}
function Metric({
  icon,
  label,
  value,
  detail,
}: {
  icon: IconName;
  label: string;
  value: number | null;
  detail: string;
}) {
  const safe = value === null ? null : Math.max(0, Math.min(100, value));
  return (
    <article className="resource-metric">
      <div className="resource-metric-heading">
        <h3>{label}</h3>
        <span className="metric-icon">
          <Icon name={icon} size={17} />
        </span>
      </div>
      <strong>
        {safe === null ? "—" : safe.toFixed(1)}
        {safe !== null && <small>%</small>}
      </strong>
      <meter aria-label={label} min={0} max={100} value={safe ?? 0} />
      <p>{detail}</p>
    </article>
  );
}
