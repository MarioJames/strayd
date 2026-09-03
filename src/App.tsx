import {
  AlertTriangle,
  ArrowRight,
  ArrowUpRight,
  Cable,
  Check,
  ChevronDown,
  CircleStop,
  Globe2,
  Laptop,
  Layers3,
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
import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { openService, scanServices, terminateService } from './api';
import type {
  ProcessOrigin,
  ResourceGroup,
  ResourceKind,
  RuntimeKind,
  ScanSnapshot,
  ServiceProcess,
  TerminateRequest,
} from './types';
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

function formatTarget(service: ServiceProcess) {
  return service.tunnelTarget
    ? `${service.tunnelTarget.host}:${service.tunnelTarget.port}`
    : null;
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
    formatTarget(service),
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

function groupSource(group: ResourceGroup) {
  return group.services.find((service) => service.resourceKind !== 'tunnel');
}

function groupTunnels(group: ResourceGroup) {
  return group.services.filter((service) => service.resourceKind === 'tunnel');
}

function isLinkedGroup(group: ResourceGroup) {
  return Boolean(groupSource(group) && groupTunnels(group).length);
}

function OriginMark({ origin }: { origin: ProcessOrigin }) {
  return origin === 'wsl' ? <TerminalSquare size={15} /> : <Laptop size={15} />;
}

function EmptyState({ query }: { query: string }) {
  return (
    <div className="empty-state">
      <div className="empty-icon"><Network size={26} /></div>
      <h2>{query ? '没有匹配的资源组' : '没有发现运行资源'}</h2>
      <p>{query ? '换一个端口、项目名、隧道或进程名试试。' : '启动开发服务或临时隧道后，Port Deck 会自动发现并关联它们。'}</p>
    </div>
  );
}

function terminateRequest(service: ServiceProcess): TerminateRequest {
  return {
    origin: service.origin,
    distribution: service.distribution,
    pid: service.pid,
    startToken: service.startToken,
    managerUnit: service.managerUnit,
  };
}

export default function App() {
  const [snapshot, setSnapshot] = useState<ScanSnapshot>({ groups: [], warnings: [], scannedAt: 0 });
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
      setSnapshot({
        ...next,
        groups: next.groups
          .map((group) => ({
            ...group,
            services: group.services.filter((service) => !stoppedIds.current.has(service.id)),
          }))
          .filter((group) => group.services.length > 0),
        scannedAt: next.scannedAt || Date.now(),
      });
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

  const services = useMemo(
    () => snapshot.groups.flatMap((group) => group.services),
    [snapshot.groups],
  );

  const filteredGroups = useMemo(() => snapshot.groups.filter((group) => {
    if (scope !== 'all' && !group.services.some((service) => service.origin === scope)) return false;
    if (resourceFilter === 'managed' && !group.services.some((service) => service.resourceKind !== 'other')) return false;
    if (
      resourceFilter !== 'all'
      && resourceFilter !== 'managed'
      && !group.services.some((service) => service.resourceKind === resourceFilter)
    ) return false;
    return group.services.some((service) => includesQuery(service, query))
      || String(group.primaryPort ?? '').includes(query.trim());
  }), [query, resourceFilter, scope, snapshot.groups]);

  const portCount = services.reduce((sum, service) => sum + service.ports.length, 0);
  const tunnelCount = services.filter((service) => service.resourceKind === 'tunnel').length;
  const linkedGroupCount = snapshot.groups.filter(isLinkedGroup).length;

  const armTermination = (key: string) => {
    if (armTimer.current) window.clearTimeout(armTimer.current);
    setArmedId(key);
    armTimer.current = window.setTimeout(() => setArmedId(null), 4_000);
  };

  const removeServices = (ids: string[]) => {
    const removed = new Set(ids);
    ids.forEach((id) => stoppedIds.current.add(id));
    setSnapshot((current) => ({
      ...current,
      groups: current.groups
        .map((group) => ({
          ...group,
          services: group.services.filter((service) => !removed.has(service.id)),
        }))
        .filter((group) => group.services.length > 0),
    }));
  };

  const stopService = async (service: ServiceProcess) => {
    const actionId = `service:${service.id}`;
    setTerminatingId(actionId);
    setArmedId(null);
    try {
      await terminateService(terminateRequest(service));
      removeServices([service.id]);
      setToast(`${service.resourceKind === 'tunnel' ? '已关闭' : '已结束'} ${resourceName(service)}`);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setTerminatingId(null);
    }
  };

  const stopGroup = async (group: ResourceGroup) => {
    const actionId = `group:${group.id}`;
    const candidates = group.services
      .filter((service) => service.canTerminate)
      .sort((left, right) => Number(right.resourceKind === 'tunnel') - Number(left.resourceKind === 'tunnel'));
    const completed: string[] = [];
    const failures: string[] = [];

    setTerminatingId(actionId);
    setArmedId(null);
    for (const service of candidates) {
      try {
        await terminateService(terminateRequest(service));
        completed.push(service.id);
      } catch (cause) {
        failures.push(`${resourceName(service)}：${cause instanceof Error ? cause.message : String(cause)}`);
      }
    }
    if (completed.length) {
      removeServices(completed);
      setToast(`已关闭 ${completed.length} 个关联资源`);
    }
    if (failures.length) {
      setError(`整组关闭完成，但有 ${failures.length} 项失败：${failures.join('；')}`);
    }
    setTerminatingId(null);
  };

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand-mark" aria-hidden="true"><span /><span /><span /></div>
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
          <div><strong>{services.length}</strong><span>个资源</span></div>
        </div>
        <div className="metric"><span>监听端口</span><strong>{portCount}</strong></div>
        <div className="metric"><span>关联组</span><strong>{linkedGroupCount}</strong></div>
        <div className="metric"><span>公网隧道</span><strong>{tunnelCount}</strong></div>
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
        {loading && snapshot.groups.length === 0 ? (
          <div className="loading-state"><LoaderCircle className="spin" size={22} />正在扫描 Windows 与 WSL…</div>
        ) : filteredGroups.length === 0 ? (
          <EmptyState query={query} />
        ) : filteredGroups.map((group, index) => {
          const source = groupSource(group);
          const tunnels = groupTunnels(group);
          const linked = Boolean(source && tunnels.length);
          const title = source ? resourceName(source) : resourceName(group.services[0]);
          const scopeService = source || group.services[0];
          const groupKind = source?.resourceKind || group.services[0].resourceKind;
          const terminableCount = group.services.filter((service) => service.canTerminate).length;
          const canStopWholeGroup = terminableCount > 1 && terminableCount === group.services.length;
          const groupActionId = `group:${group.id}`;
          const railLabel = linked ? 'LINKED PORT' : tunnels.length ? 'TARGET PORT' : 'LOCAL PORT';

          return (
            <article
              className={`service-card group-card origin-${scopeService.origin} kind-${groupKind}${linked ? ' linked-group' : ''}`}
              key={group.id}
              style={{ '--row-index': index } as React.CSSProperties}
            >
              <div className="resource-rail">
                <span className="resource-label">{railLabel}</span>
                <strong>{group.primaryPort ? `:${group.primaryPort}` : 'ACTIVE'}</strong>
                <span className="group-size">{group.services.length} 项资源</span>
              </div>

              <div className="group-main">
                <div className="group-heading">
                  <div>
                    <div className="title-line">
                      <h2>{title}</h2>
                      {linked
                        ? <span className="link-badge"><Layers3 size={12} />已关联</span>
                        : <span className={`runtime runtime-${group.services[0].runtime}`}>{runtimeLabels[group.services[0].runtime]}</span>}
                    </div>
                    <div className="source-line">
                      <span><OriginMark origin={scopeService.origin} />{scopeService.origin === 'wsl' ? `WSL · ${scopeService.distribution}` : 'Windows'}</span>
                      <span>{linked ? `${tunnels.length} 条隧道关联本地服务` : `${group.services.length} 项独立资源`}</span>
                    </div>
                  </div>
                  {canStopWholeGroup && (
                    armedId === groupActionId ? (
                      <button className="stop-button group-stop confirm" onClick={() => void stopGroup(group)} disabled={terminatingId === groupActionId}>
                        {terminatingId === groupActionId ? <LoaderCircle className="spin" size={16} /> : <CircleStop size={16} />}
                        确认关闭 {terminableCount} 项
                      </button>
                    ) : (
                      <button className="stop-button group-stop" onClick={() => armTermination(groupActionId)}>
                        <CircleStop size={16} />关闭整组
                      </button>
                    )
                  )}
                </div>

                {linked && source && (
                  <div className="route-map" aria-label={`公网流量通过隧道转发到本地 ${group.primaryPort} 端口`}>
                    <span className="route-node public-node"><Globe2 size={14} />公网入口</span>
                    <ArrowRight size={14} />
                    <span className="route-node tunnel-node"><Cable size={14} />{tunnels.length === 1 ? runtimeLabels[tunnels[0].runtime] : `${tunnels.length} 条隧道`}</span>
                    <ArrowRight size={14} />
                    <span className="route-node local-node"><Server size={14} /><strong>:{group.primaryPort}</strong>{runtimeLabels[source.runtime]}</span>
                  </div>
                )}

                <div className="member-list">
                  {group.services.map((service) => {
                    const serviceActionId = `service:${service.id}`;
                    const isTunnel = service.resourceKind === 'tunnel';
                    const canOpen = service.resourceKind === 'development' && service.ports.length > 0;
                    const target = formatTarget(service);
                    const individualLabel = linked
                      ? isTunnel ? '仅关隧道' : '仅关服务'
                      : isTunnel ? '关闭隧道' : '结束进程';

                    return (
                      <div className={`member-row kind-${service.resourceKind}`} key={service.id}>
                        <div className="member-marker">{isTunnel ? <Cable size={15} /> : <Server size={15} />}</div>
                        <div className="member-content">
                          <div className="member-title">
                            <span className="member-role">{isTunnel ? '公网隧道' : service.resourceKind === 'system' ? '系统服务' : '源服务'}</span>
                            <strong>{runtimeLabels[service.runtime]}</strong>
                            <span>PID {service.pid}</span>
                          </div>
                          <div className="member-status">
                            {isTunnel
                              ? <span>{target ? `代理到 ${target}` : '目标端口未知'}{!linked && target ? ' · 未发现对应监听进程' : ''}</span>
                              : <span>{service.ports.length ? `监听 ${service.ports.map((port) => `:${port}`).join(' · ')}` : '没有监听端口'}</span>}
                            {service.managerUnit && <span className="manager-label"><Settings2 size={12} />{service.managerUnit}</span>}
                          </div>
                          <code className="member-command" title={service.command}>{service.command}</code>
                        </div>
                        <div className="member-actions">
                          {canOpen && (
                            <button className="icon-button" title="在浏览器中打开" aria-label={`打开 ${service.ports[0]} 端口`} onClick={() => void openService(service.ports[0])}>
                              <ArrowUpRight size={17} />
                            </button>
                          )}
                          {!service.canTerminate ? (
                            <span className="protected-status" title="为避免断开远程连接，Port Deck 不会结束 sshd">
                              <ShieldCheck size={15} />受保护
                            </span>
                          ) : armedId === serviceActionId ? (
                            <button className="stop-button confirm" onClick={() => void stopService(service)} disabled={terminatingId !== null}>
                              {terminatingId === serviceActionId ? <LoaderCircle className="spin" size={16} /> : <CircleStop size={16} />}
                              确认{individualLabel}
                            </button>
                          ) : (
                            <button className="stop-button" onClick={() => armTermination(serviceActionId)} disabled={terminatingId !== null}>
                              <CircleStop size={16} />{individualLabel}
                            </button>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            </article>
          );
        })}
      </section>

      <footer>
        <span>每 5 秒自动扫描</span><span className="footer-divider" /><span>关闭窗口后仍在托盘运行</span>
      </footer>
      {toast && <div className="toast" role="status"><Check size={17} />{toast}</div>}
    </main>
  );
}
