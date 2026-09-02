import {
  AlertTriangle,
  ArrowUpRight,
  Box,
  Check,
  ChevronDown,
  CircleStop,
  Laptop,
  LoaderCircle,
  Network,
  RefreshCw,
  Search,
  Server,
  TerminalSquare,
  X,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { openService, scanServices, terminateService } from './api';
import type { ProcessOrigin, RuntimeKind, ScanSnapshot, ServiceProcess } from './types';
import './styles.css';

type Scope = 'all' | ProcessOrigin;
type ServiceFilter = 'all' | 'dev';

const runtimeLabels: Record<RuntimeKind, string> = {
  nextJs: 'Next.js',
  vite: 'Vite',
  nuxt: 'Nuxt',
  astro: 'Astro',
  svelteKit: 'SvelteKit',
  remix: 'Remix',
  angular: 'Angular',
  storybook: 'Storybook',
  webpack: 'Webpack',
  parcel: 'Parcel',
  rspack: 'Rspack',
  node: 'Node.js',
  bun: 'Bun',
  deno: 'Deno',
  other: '其他服务',
};

function formatScanTime(timestamp: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(timestamp);
}

function includesQuery(service: ServiceProcess, query: string) {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return true;

  return [
    service.projectName,
    service.distribution,
    service.processName,
    service.command,
    service.cwd,
    runtimeLabels[service.runtime],
    ...service.ports.map(String),
  ].some((value) => value?.toLocaleLowerCase().includes(needle));
}

function OriginMark({ origin }: { origin: ProcessOrigin }) {
  return origin === 'wsl' ? <TerminalSquare size={15} /> : <Laptop size={15} />;
}

function EmptyState({ query }: { query: string }) {
  return (
    <div className="empty-state">
      <div className="empty-icon"><Network size={26} /></div>
      <h2>{query ? '没有匹配的监听端口' : '没有发现监听服务'}</h2>
      <p>{query ? '换一个端口、项目名或进程名试试。' : '启动开发服务后，Port Deck 会自动发现它。'}</p>
    </div>
  );
}

export default function App() {
  const [snapshot, setSnapshot] = useState<ScanSnapshot>({ services: [], warnings: [], scannedAt: 0 });
  const [scope, setScope] = useState<Scope>('all');
  const [serviceFilter, setServiceFilter] = useState<ServiceFilter>('dev');
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [armedId, setArmedId] = useState<string | null>(null);
  const [terminatingId, setTerminatingId] = useState<string | null>(null);
  const stoppedIds = useRef<Set<string>>(new Set());
  const [toast, setToast] = useState<string | null>(null);
  const armTimer = useRef<number | null>(null);

  const refresh = useCallback(async (quiet = false) => {
    if (!quiet) setLoading(true);
    try {
      const next = await scanServices();
      setSnapshot((current) => ({
        ...next,
        services: next.services.filter((service) => !stoppedIds.current.has(service.id)),
        scannedAt: next.scannedAt || Date.now(),
      }));
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(true), 5_000);
    let unlisten: (() => void) | undefined;

    if ('__TAURI_INTERNALS__' in window) {
      void listen('refresh-requested', () => void refresh()).then((cleanup) => {
        unlisten = cleanup;
      });
    }

    return () => {
      window.clearInterval(timer);
      unlisten?.();
      if (armTimer.current) window.clearTimeout(armTimer.current);
    };
  }, [refresh]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 2_600);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const filteredServices = useMemo(() => snapshot.services.filter((service) => {
    if (scope !== 'all' && service.origin !== scope) return false;
    if (serviceFilter === 'dev' && !service.isDevServer && !query.trim()) return false;
    return includesQuery(service, query);
  }), [query, scope, serviceFilter, snapshot.services]);

  const portCount = snapshot.services.reduce((sum, service) => sum + service.ports.length, 0);
  const wslCount = snapshot.services.filter((service) => service.origin === 'wsl').length;
  const windowsCount = snapshot.services.length - wslCount;

  const armTermination = (service: ServiceProcess) => {
    if (armTimer.current) window.clearTimeout(armTimer.current);
    setArmedId(service.id);
    armTimer.current = window.setTimeout(() => setArmedId(null), 4_000);
  };

  const stopService = async (service: ServiceProcess) => {
    setTerminatingId(service.id);
    setArmedId(null);
    try {
      await terminateService({
        origin: service.origin,
        distribution: service.distribution,
        pid: service.pid,
        startToken: service.startToken,
      });
      stoppedIds.current.add(service.id);
      setSnapshot((current) => ({
        ...current,
        services: current.services.filter((item) => item.id !== service.id),
      }));
      setToast(`已结束 ${service.projectName || service.processName}`);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setTerminatingId(null);
    }
  };

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand-mark" aria-hidden="true">
          <span /><span /><span />
        </div>
        <div className="brand-copy">
          <h1>Port Deck</h1>
          <p>本地开发端口控制台</p>
        </div>
        <button className="refresh-button" onClick={() => void refresh()} disabled={loading}>
          <RefreshCw size={16} className={loading ? 'spin' : ''} />
          {loading ? '扫描中' : '重新扫描'}
        </button>
      </header>

      <section className="status-ribbon" aria-label="扫描摘要">
        <div className="signal-block">
          <span className="live-dot" />
          <div>
            <strong>{snapshot.services.length}</strong>
            <span>个服务</span>
          </div>
        </div>
        <div className="metric"><span>监听端口</span><strong>{portCount}</strong></div>
        <div className="metric"><span>WSL</span><strong>{wslCount}</strong></div>
        <div className="metric"><span>Windows</span><strong>{windowsCount}</strong></div>
        <div className="scan-time">
          {snapshot.scannedAt ? `最后扫描 ${formatScanTime(snapshot.scannedAt)}` : '正在建立进程索引'}
        </div>
      </section>

      <section className="toolbar" aria-label="筛选服务">
        <label className="search-box">
          <Search size={17} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索端口、项目、进程或路径"
            aria-label="搜索服务"
          />
          {query && <button onClick={() => setQuery('')} aria-label="清除搜索"><X size={15} /></button>}
        </label>
        <div className="segmented" aria-label="环境范围">
          {(['all', 'wsl', 'windows'] as const).map((item) => (
            <button key={item} className={scope === item ? 'active' : ''} onClick={() => setScope(item)}>
              {item === 'all' ? '全部环境' : item === 'wsl' ? 'WSL' : 'Windows'}
            </button>
          ))}
        </div>
        <label className="select-wrap">
          <select value={serviceFilter} onChange={(event) => setServiceFilter(event.target.value as ServiceFilter)}>
            <option value="all">全部端口</option>
            <option value="dev">仅开发服务</option>
          </select>
          <ChevronDown size={14} />
        </label>
      </section>

      {error && (
        <div className="notice error-notice" role="alert">
          <AlertTriangle size={17} />
          <span>{error}</span>
          <button onClick={() => setError(null)} aria-label="关闭错误"><X size={15} /></button>
        </div>
      )}

      {snapshot.warnings.map((warning) => (
        <div className="notice" key={warning}><AlertTriangle size={16} /><span>{warning}</span></div>
      ))}

      <section className="service-list" aria-live="polite" aria-busy={loading}>
        {loading && snapshot.services.length === 0 ? (
          <div className="loading-state"><LoaderCircle className="spin" size={22} />正在扫描 Windows 与 WSL…</div>
        ) : filteredServices.length === 0 ? (
          <EmptyState query={query} />
        ) : (
          filteredServices.map((service, index) => (
            <article className={`service-card origin-${service.origin}`} key={service.id} style={{ '--row-index': index } as React.CSSProperties}>
              <div className="port-rail">
                <span className="port-label">PORT</span>
                <strong>:{service.ports[0]}</strong>
                {service.ports.length > 1 && <span className="extra-ports">+{service.ports.length - 1}</span>}
              </div>

              <div className="service-main">
                <div className="service-heading">
                  <div>
                    <div className="title-line">
                      <h2>{service.projectName || service.processName}</h2>
                      <span className={`runtime runtime-${service.runtime}`}>{runtimeLabels[service.runtime]}</span>
                    </div>
                    <div className="source-line">
                      <span><OriginMark origin={service.origin} />{service.origin === 'wsl' ? `WSL · ${service.distribution}` : 'Windows'}</span>
                      <span>PID {service.pid}</span>
                      {service.ports.length > 1 && <span>{service.ports.join(' · ')}</span>}
                    </div>
                  </div>
                  <div className="card-actions">
                    <button className="icon-button" title="在浏览器中打开" aria-label={`打开 ${service.ports[0]} 端口`} onClick={() => void openService(service.ports[0])}>
                      <ArrowUpRight size={17} />
                    </button>
                    {armedId === service.id ? (
                      <button className="stop-button confirm" onClick={() => void stopService(service)} disabled={terminatingId === service.id}>
                        {terminatingId === service.id ? <LoaderCircle className="spin" size={16} /> : <CircleStop size={16} />}
                        确认结束
                      </button>
                    ) : (
                      <button className="stop-button" onClick={() => armTermination(service)}>
                        <CircleStop size={16} />结束进程
                      </button>
                    )}
                  </div>
                </div>
                <div className="process-details">
                  <div><Box size={14} /><code title={service.cwd || ''}>{service.cwd || '工作目录不可用'}</code></div>
                  <div><Server size={14} /><code title={service.command}>{service.command}</code></div>
                </div>
              </div>
            </article>
          ))
        )}
      </section>

      <footer>
        <span>每 5 秒自动扫描</span>
        <span className="footer-divider" />
        <span>关闭窗口后仍在托盘运行</span>
      </footer>

      {toast && <div className="toast" role="status"><Check size={17} />{toast}</div>}
    </main>
  );
}
