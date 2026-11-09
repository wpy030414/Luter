import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router';
import {
  AlertCircle,
  ArrowRight,
  Check,
  GitBranch,
  KeyRound,
  Loader2,
  Plug,
  RefreshCw,
} from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from './ui/dialog';
import { Button } from './ui/button';
import { marketApi, pluginsApi, type MarketPlugin } from '@/lib/api';
import { beginAutoInstall, endAutoInstall } from '@/lib/pluginAutoInstall';
import { useT } from '@/i18n';
import { cn } from '@/lib/utils';

type Tab = 'byok' | 'market';
type Phase = 'idle' | 'installing' | 'logining' | 'starting' | 'enabling' | 'done';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 安装完成且插件已启用后回调（父页面刷新供应商/插件状态） */
  onInstalled?: () => void;
}

const norm = (p: string) => p.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
const repoOf = (workDir: string) => workDir.replace(/\\/g, '/').split('/').filter(Boolean).pop() || '';

/** 轮询插件列表，直到该 work_dir 对应的插件注册并变为 active */
async function waitForActive(workDir: string, timeoutMs = 120_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  const target = norm(workDir);
  while (Date.now() < deadline) {
    const list = await pluginsApi.list();
    const hit = list.find((p) => p.work_dir && norm(p.work_dir) === target);
    if (hit?.status === 'active') return;
    await new Promise((r) => setTimeout(r, 1500));
  }
  throw new Error('timeout');
}

export function AddProviderDialog({ open, onOpenChange, onInstalled }: Props) {
  const t = useT();
  const navigate = useNavigate();

  const [tab, setTab] = useState<Tab>('byok');
  const [items, setItems] = useState<MarketPlugin[]>([]);
  const [installedRepos, setInstalledRepos] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>('idle');

  const loadMarket = useCallback(
    async (force = false) => {
      setLoading(true);
      setError(null);
      try {
        const [market, plugins] = await Promise.all([marketApi.search(force), pluginsApi.list()]);
        setItems(market.items);
        setInstalledRepos(
          new Set(
            plugins
              .map((p) => (p.work_dir ? repoOf(p.work_dir) : ''))
              .filter((r): r is string => Boolean(r)),
          ),
        );
      } catch (e: any) {
        setError(t('providers.addDialog.market.load_failed', { msg: e?.message || String(e) }));
      } finally {
        setLoading(false);
      }
    },
    [t],
  );

  // 打开时重置；切到「反向代理」时按需加载列表
  useEffect(() => {
    if (!open) return;
    setTab('byok');
    setError(null);
    setPhase('idle');
    setInstalling(null);
  }, [open]);

  useEffect(() => {
    if (open && tab === 'market' && items.length === 0 && !loading) void loadMarket();
  }, [open, tab, items.length, loading, loadMarket]);

  const handleByok = () => {
    onOpenChange(false);
    navigate('/providers/new');
  };

  const handleInstall = async (plugin: MarketPlugin) => {
    if (installing) return;
    setInstalling(plugin.repo);
    setError(null);
    setPhase('installing');
    // 让后续的 plugin-register 事件静默 confirm，不弹确认框
    beginAutoInstall();
    try {
      const res = await marketApi.install(plugin.repo);
      // 插件网关没有凭证时照样监听端口但请求全 401，所以必须先登录再 serve。
      // login 是阻塞请求：后端跑完 pnpm run login 才返回，并在成功后就地拉起 serve。
      if (res.login_script) {
        setPhase('logining');
        await marketApi.login(res.work_dir);
      }
      setPhase('starting');
      setPhase('enabling');
      await waitForActive(res.work_dir);
      setPhase('done');
      onInstalled?.();
      setTimeout(() => onOpenChange(false), 700);
    } catch (e: any) {
      const msg = e?.message === 'timeout' ? t('providers.addDialog.market.timeout') : e?.message || String(e);
      setError(t('providers.addDialog.market.install_failed', { msg }));
      setPhase('idle');
    } finally {
      endAutoInstall();
      setInstalling(null);
    }
  };

  const phaseLabel = (): string => {
    switch (phase) {
      case 'installing':
        return t('providers.addDialog.market.installing');
      case 'logining':
        return t('providers.addDialog.market.logining');
      case 'starting':
        return t('providers.addDialog.market.starting');
      case 'enabling':
        return t('providers.addDialog.market.enabling');
      case 'done':
        return t('providers.addDialog.market.done');
      default:
        return t('providers.addDialog.market.install');
    }
  };

  return (
    <Dialog open={open} onOpenChange={(v) => !installing && onOpenChange(v)}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t('providers.addDialog.title')}</DialogTitle>
          <DialogDescription>{t('providers.addDialog.desc')}</DialogDescription>
        </DialogHeader>

        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
          <button
            type="button"
            onClick={handleByok}
            className={cn(
              'group text-left rounded-lg border p-4 transition-colors',
              'hover:border-primary/60 hover:bg-accent/40',
              tab === 'byok' ? 'border-primary/60 bg-accent/30' : 'border-border',
            )}
          >
            <div className="flex items-center gap-2 mb-2">
              <KeyRound className="w-4 h-4 text-primary" />
              <span className="font-medium">{t('providers.addDialog.byok.title')}</span>
              <ArrowRight className="w-3.5 h-3.5 ml-auto text-muted-foreground group-hover:text-primary" />
            </div>
            <p className="text-xs text-muted-foreground leading-relaxed">
              {t('providers.addDialog.byok.desc')}
            </p>
          </button>

          <button
            type="button"
            onClick={() => setTab('market')}
            className={cn(
              'group text-left rounded-lg border p-4 transition-colors',
              'hover:border-primary/60 hover:bg-accent/40',
              tab === 'market' ? 'border-primary/60 bg-accent/30' : 'border-border',
            )}
          >
            <div className="flex items-center gap-2 mb-2">
              <Plug className="w-4 h-4 text-primary" />
              <span className="font-medium">{t('providers.addDialog.market.title')}</span>
              <ArrowRight className="w-3.5 h-3.5 ml-auto text-muted-foreground group-hover:text-primary" />
            </div>
            <p className="text-xs text-muted-foreground leading-relaxed">
              {t('providers.addDialog.market.desc')}
            </p>
          </button>
        </div>

        {tab === 'market' && (
          <div className="space-y-3">
            <div className="flex items-center justify-between gap-2">
              <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <GitBranch className="w-3.5 h-3.5" />
                {t('providers.addDialog.market.source')}
              </span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => void loadMarket(true)}
                disabled={loading || Boolean(installing)}
              >
                <RefreshCw className={cn('w-3.5 h-3.5 mr-1.5', loading && 'animate-spin')} />
                {t('providers.addDialog.market.refresh')}
              </Button>
            </div>

            {error && (
              <div className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-3 text-xs text-destructive">
                <AlertCircle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
                <span className="break-all">{error}</span>
              </div>
            )}

            {phase === 'logining' && (
              <div className="rounded-md border border-primary/40 bg-primary/5 p-3 text-xs text-primary">
                {t('providers.addDialog.market.login_hint')}
              </div>
            )}

            <div className="max-h-[320px] overflow-y-auto space-y-2 pr-1">
              {loading && items.length === 0 ? (
                <div className="flex items-center justify-center py-10">
                  <Loader2 className="w-5 h-5 animate-spin text-muted-foreground" />
                </div>
              ) : items.length === 0 ? (
                <p className="py-10 text-center text-sm text-muted-foreground">
                  {t('providers.addDialog.market.empty')}
                </p>
              ) : (
                items.map((item) => {
                  const isInstalled = installedRepos.has(item.repo);
                  const isBusy = installing === item.repo;
                  return (
                    <div
                      key={item.repo}
                      className="flex items-start gap-3 rounded-md border border-border p-3"
                    >
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="font-medium text-sm">{item.display_name}</span>
                          <span className="text-[11px] text-muted-foreground truncate">
                            {item.repo}
                          </span>
                          {isInstalled && (
                            <span className="flex items-center gap-0.5 text-[11px] text-emerald-600">
                              <Check className="w-3 h-3" />
                              {t('providers.addDialog.market.installed')}
                            </span>
                          )}
                        </div>
                        <p className="text-xs text-muted-foreground mt-1 line-clamp-2">
                          {item.description || t('providers.addDialog.market.no_desc')}
                        </p>
                        {item.pushed_at && (
                          <p className="text-[11px] text-muted-foreground/70 mt-1">
                            {t('providers.addDialog.market.updated', {
                              date: item.pushed_at.slice(0, 10),
                            })}
                          </p>
                        )}
                      </div>
                      <Button
                        size="sm"
                        variant={isInstalled ? 'outline' : 'default'}
                        disabled={Boolean(installing)}
                        onClick={() => void handleInstall(item)}
                      >
                        {isBusy ? (
                          <>
                            <Loader2 className="w-3.5 h-3.5 mr-1.5 animate-spin" />
                            {phaseLabel()}
                          </>
                        ) : isInstalled ? (
                          t('providers.addDialog.market.reinstall')
                        ) : (
                          t('providers.addDialog.market.install')
                        )}
                      </Button>
                    </div>
                  );
                })
              )}
            </div>

            <p className="text-[11px] text-muted-foreground">
              {t('providers.addDialog.market.requires')}
            </p>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
