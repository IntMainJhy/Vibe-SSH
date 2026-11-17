import { useEffect, useState } from "react";
import { call, explain } from "./api";
import { Icon } from "./Icon";
import { bytes } from "./Transfers";

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
  const [series, setSeries] = useState<number[]>([]);
  const [at, setAt] = useState<Date>();
  const [error, setError] = useState("");
  const [refresh, setRefresh] = useState(0);
  const [sampling, setSampling] = useState(false);
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout>;
    const sample = async () => {
      if (!alive) return;
      if (document.hidden) {
        timer = setTimeout(() => void sample(), 5000);
        return;
      }
      setSampling(true);
      try {
        const result = await call<Sample>("resources", { space });
        if (!alive) return;
        setSample(result);
        setAt(new Date());
        setError("");
        if (result.cpu_percent !== null)
          setSeries((old) => [...old.slice(-39), result.cpu_percent!]);
        timer = setTimeout(() => void sample(), 5000);
      } catch (e) {
        if (alive) setError(explain(e));
      } finally {
        if (alive) setSampling(false);
      }
    };
    void sample();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [space, refresh]);
  const memory = sample ? sample.memory_total - sample.memory_available : 0;
  return (
    <section className="resources-page">
      <header className="resource-heading">
        <div>
          <h2>服务器概览</h2>
          <p>
            {at
              ? `上次采样 ${at.toLocaleTimeString("zh-CN")}`
              : "正在读取服务器状态"}{" "}
            · Linux
          </p>
        </div>
        <button disabled={sampling} onClick={() => setRefresh((n) => n + 1)}>
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
          label="CPU 使用率"
          value={sample?.cpu_percent ?? null}
          detail={sample ? `${sample.cores} 个逻辑核心` : "等待采样"}
        />
        <Metric
          label="内存使用率"
          value={sample ? percent(memory, sample.memory_total) : null}
          detail={
            sample
              ? `${bytes(memory)} / ${bytes(sample.memory_total)}`
              : "等待采样"
          }
        />
        <Metric
          label="存储使用率"
          value={sample ? percent(sample.disk_used, sample.disk_total) : null}
          detail={
            sample
              ? `${bytes(sample.disk_used)} / ${bytes(sample.disk_total)} · 根分区 /`
              : "等待采样"
          }
        />
      </div>
      <section className="cpu-history">
        <div>
          <h3>CPU 采样趋势</h3>
          <span>仅记录当前页面打开期间</span>
        </div>
        <div className="resource-chart">
          <span className="chart-max">100%</span>
          <span className="chart-min">0%</span>
          {series.length > 1 ? (
            <svg
              viewBox="0 0 600 140"
              preserveAspectRatio="none"
              role="img"
              aria-label={`最近 ${series.length} 次 CPU 采样，最新 ${series.at(-1)?.toFixed(1)}%`}
            >
              <path
                d={`M ${series.map((n, i) => `${(i / (series.length - 1)) * 600},${140 - Math.max(0, Math.min(100, n)) * 1.4}`).join(" L ")}`}
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
      </section>
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
  label,
  value,
  detail,
}: {
  label: string;
  value: number | null;
  detail: string;
}) {
  const safe = value === null ? null : Math.max(0, Math.min(100, value));
  return (
    <article className="resource-metric">
      <h3>{label}</h3>
      <strong>
        {safe === null ? "—" : safe.toFixed(1)}
        {safe !== null && <small>%</small>}
      </strong>
      <meter aria-label={label} min={0} max={100} value={safe ?? 0} />
      <p>{detail}</p>
    </article>
  );
}
