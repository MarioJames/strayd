import {
  AlertTriangle,
  ArrowUpRight,
  Box,
  Cable,
  Check,
  ChevronDown,
  CircleStop,
  Laptop,
  LoaderCircle,
  Network,
  RefreshCw,
  Search,
  Server,
  Settings2,
  ShieldCheck,
  TerminalSquare,
  X,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { openService, scanServices, terminateService } from './api';
import type { ProcessOrigin, ResourceKind, RuntimeKind, ScanSnapshot, ServiceProcess } from './types';
import './styles.css';

type Scope = 'all' | ProcessOrigin;
type ResourceFilter = 'managed' | ResourceKind | 'all';

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
  cloudflared: 'Cloudflare Tunnel',
  ngrok: 'ngrok',
  sshTunnel: 'SSH 反向隧道',
  frp: 'frp Client',
  localTunnel: 'localtunnel',
  bore: 'bore',
  sshd: 'SSH Server',
  other: '其他服务',
};

const resourceLabels: Record<ResourceKind, string> = {
  development: '开发服务',
  tunnel: '公网隧道',
  system: '系统服务',
  other: '其他进程',
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
    service.managerUnit,
    runtimeLabels[service.runtime],
    resourceLabels[service.resourceKind],
    ...service.ports.map(String),
  ].some((value) => value?.toLocaleLowerCase().includes(needle));
}

function resourceName(service: ServiceProcess) {
  if (service.resourceKind === 'system') return runtimeLabels[service.runtime];
  if (service.resourceKind === 'tunnel') {
    if (service.managerUnit) return service.managerUnit.replace(/\.service$/, '');
    if (service.projectName) return `${service.projectName} 隧道`;
    return runtimeLabels[service.runtime];
  }
  return service.projectName || service.processName;
}

function railContent(service: ServiceProcess) {
  if (service.resourceKind === 'tunnel') return { label: 'PUBLIC', value: 'TUNNEL' };
  if (service.resourceKind === 'system') {
    return { label: 'SYSTEM', value: service.ports[0] ? `:${service.ports[0]}` : 'SSHD' };
  }
  return {
    label: service.ports.length ? 'PORT' : 'PROCESS',
    value: service.ports[0] ? `:${service.ports[0]}` : 'ACTIVE',
  };
}

function OriginMark({ origin }: { origin: ProcessOrigin }) {
  return origin === 'wsl' ? <TerminalSquare size={15} /> : <Laptop size={15} />;
}

function EmptyState({ query }: { query: string }) {
  return (
    <div className="empty-state">
      <div className="empty-icon"><Network size={26} /></div>
      <h2>{query ? '没有匹配的运行资源' : '没有发现运行资源'}</h2>
      <p>{query ? '换一个端口、项目名、隧道或进程名试试。' : '启动开发服务或临时隧道后，Port Deck 会自动发现它。'}</p>
    </div>
  );
}

export default function App() {
  const [snapshot, setSnapshot] = useState<ScanSnapshot>({ services: [], warnings: [], scannedAt: 0 });
  const [scope, setScope] = useState<Scope>('all');
  const [resourceFilter, setResourceFilter] = useState<ResourceFilter>('managed');
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
    if (resourceFilter === 'managed' && service.resourceKind === 'other') return false;
    if (resourceFilter !== 'all' && resourceFilter !== 'managed' && service.resourceKind !== resourceFilter) return false;
    return includesQuery(service, query);
  }), [query, resourceFilter, scope, snapshot.services]);

  const portCount = snapshot.services.reduce((sum, service) => sum + service.ports.length, 0);
  const tunnelCount = snapshot.services.filter((service) => service.resourceKind === 'tunnel').length;
  const sshdCount = snapshot.services.filter((service) => service.runtime === 'sshd').length;

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
        managerUnit: service.managerUnit,
      });
      stoppedIds.current.add(service.id);
      setSnapshot((current) => ({
        ...current,
        services: current.services.filter((item) => item.id !== service.id),
      }));
      setToast(`${service.resourceKind === 'tunnel' ? '已关闭' : '已结束'} ${resourceName(service)}`);
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
          <p>本地运行资源控制台</p>
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
            <span>个资源</span>
          </div>
        </div>
        <div className="metric"><span>监听端口</span><strong>{portCount}</strong></div>
        <div className="metric"><span>公网隧道</span><strong>{tunnelCount}</strong></div>
        <div className="metric"><span>SSH 服务</span><strong>{sshdCount}</strong></div>
        <div className="scan-time">
          {snapshot.scannedAt ? `最后扫描 ${formatScanTime(snapshot.scannedAt)}` : '正在建立进程索引'}
        </div>
      </section>

      <section className="toolbar" aria-label="筛选运行资源">
        <label className="search-box">
          <Search size={17} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索端口、项目、隧道、进程或路径"
            aria-label="搜索运行资源"
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
          <select value={resourceFilter} onChange={(event) => setResourceFilter(event.target.value as ResourceFilter)}>
            <option value="managed">主要资源</option>
            <option value="development">开发服务</option>
            <option value="tunnel">公网隧道</option>
            <option value="system">系统服务</option>
            <option value="all">全部进程</option>
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
          filteredServices.map((service, index) => {
            const rail = railContent(service);
            const canOpen = service.resourceKind === 'development' && service.ports.length > 0;
            const stopLabel = service.resourceKind === 'tunnel' ? '关闭隧道' : '结束进程';
            return (
            <article className={`service-card origin-${service.origin} kind-${service.resourceKind}`} key={service.id} style={{ '--row-index': index } as React.CSSProperties}>
              <div className="resource-rail">
                <span className="resource-label">{rail.label}</span>
                <strong>{rail.value}</strong>
                {service.ports.length > 1 && <span className="extra-ports">+{service.ports.length - 1}</span>}
              </div>

              <div className="service-main">
                <div className="service-heading">
                  <div>
                    <div className="title-line">
                      <h2>{resourceName(service)}</h2>
                      <span className={`runtime runtime-${service.runtime}`}>{runtimeLabels[service.runtime]}</span>
                    </div>
                    <div className="source-line">
                      <span><OriginMark origin={service.origin} />{service.origin === 'wsl' ? `WSL · ${service.distribution}` : 'Windows'}</span>
                      <span>PID {service.pid}</span>
                      {service.ports.length > 1 && <span>端口 {service.ports.join(' · ')}</span>}
                    </div>
                  </div>
                  <div className="card-actions">
                    {canOpen && <button className="icon-button" title="在浏览器中打开" aria-label={`打开 ${service.ports[0]} 端口`} onClick={() => void openService(service.ports[0])}>
                      <ArrowUpRight size={17} />
                    </button>}
                    {!service.canTerminate ? (
                      <span className="protected-status" title="为避免断开远程连接，Port Deck 不会结束 sshd">
                        <ShieldCheck size={15} />受保护
                      </span>
                    ) : armedId === service.id ? (
                      <button className="stop-button confirm" onClick={() => void stopService(service)} disabled={terminatingId === service.id}>
                        {terminatingId === service.id ? <LoaderCircle className="spin" size={16} /> : <CircleStop size={16} />}
                        确认{service.resourceKind === 'tunnel' ? '关闭' : '结束'}
                      </button>
                    ) : (
                      <button className="stop-button" onClick={() => armTermination(service)}>
                        <CircleStop size={16} />{stopLabel}
                      </button>
                    )}
                  </div>
                </div>
                <div className="process-details">
                  {service.managerUnit && <div className="manager-detail"><Settings2 size={14} /><code title={service.managerUnit}>systemd · {service.managerUnit}</code></div>}
                  <div><Box size={14} /><code title={service.cwd || ''}>{service.cwd || '工作目录不可用'}</code></div>
                  <div>{service.resourceKind === 'tunnel' ? <Cable size={14} /> : <Server size={14} />}<code title={service.command}>{service.command}</code></div>
                </div>
              </div>
            </article>
          );})
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
