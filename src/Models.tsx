import {useState, useEffect} from 'react';
import {call, type Row, type Action} from './api';

function modelSubtitle(m: Row, active: boolean) {
  if (m.fits === false) return `Needs ${m.ram} GB RAM`;
  return active || m.cached || m.installedCopy
    ? 'Ready on this Mac'
    : `${(m.total / 1e9).toFixed(1)} GB download`;
}

export function Models({
  model,
  act,
  simple = false,
}: {
  model: Row | null;
  act: Action;
  simple?: boolean;
}) {
  const [fit, fitting] = useState<Row | null>(null);
  const [download, setDownload] = useState<Row | null>(null);
  const [items, set] = useState<Row[] | null>(null),
    [working, busy] = useState(''),
    [notice, note] = useState('');

  async function refresh() {
    try {
      set(await call('models'));
    } catch (e) {
      note(String(e));
    }
  }
  useEffect(() => {
    void refresh();
    call('fit')
      .then(fitting)
      .catch(e => note(String(e)));
  }, []);
  useEffect(() => {
    let live = true;
    let timer: number | undefined;
    async function check() {
      try {
        const progress = await call('modelDownloadStatus');
        if (live) setDownload(progress);
      } catch (error) {
        if (live) note(String(error));
      }
      if (live && working === 'gguf') timer = window.setTimeout(check, 800);
    }
    void check();
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [working]);

  const [downloading, setDownloading] = useState('');
  const choices: Row[] = download?.choices ?? (download ? [download] : []);
  const current = (model?.name || '').replace(/\.gguf$/, '');
  const externalModels = (items || []).filter(
    m =>
      (!m.bytes || !fit?.memory || m.bytes * 1.3 + 3e9 < fit.memory) &&
      !choices.some(c => c.file?.replace(/\.gguf$/, '') === m.name),
  );

  async function selectModel(op: string, args: Row, label: string) {
    setDownloading(label);
    busy(op === 'downloadBundled' ? 'gguf' : label);
    note('');
    try {
      const selected = await act(op, args);
      if (selected?.name) note(`Now using ${op === 'importModel' ? selected.name : label}.`);
    } catch (e) {
      note(String(e));
    } finally {
      busy('');
    }
  }

  return (
    <section aria-label="Local models" className="model-list">
      {choices.map(m => {
        const active = current === m.file?.replace(/\.gguf$/, '');
        const existing = m.cached || m.installedCopy;
        const space = existing || (fit?.freeDisk ?? 0) >= m.required;
        const isLoading = working === 'gguf' && downloading === m.label;
        return (
          <div className="row" key={m.label}>
            <span className="model-meta">
              <span className="perm-name">{m.label}</span>
              <small>{modelSubtitle(m, active)}</small>
            </span>
            {active ? (
              <span className="perm-badge granted">✓ Active</span>
            ) : (
              <button
                type="button"
                className="btn-sm"
                disabled={!!working || !space || m.fits === false}
                onClick={() =>
                  void selectModel(m.installedCopy && !m.cached ? 'useInstalledModel' : 'downloadBundled', {label: m.label}, m.label)
                }
              >
                {isLoading
                  ? 'Installing…'
                  : !space
                    ? 'Low disk'
                    : existing
                      ? 'Use'
                      : m.bytes
                        ? 'Resume'
                        : 'Install'}
              </button>
            )}
          </div>
        );
      })}

      {externalModels.filter(m => current === m.name).map(m => (
        <div className="row" key={m.name}>
          <span className="model-meta">
            <span className="perm-name">{m.name}</span>
            <small>via {m.runtime === 'Sumi' ? 'Beside' : m.runtime || 'Local'}</small>
          </span>
          <span className="perm-badge granted">✓ Active</span>
        </div>
      ))}

      {externalModels.filter(m => current !== m.name).length > 0 && (
        <details>
          <summary>
            More on this Mac ({externalModels.filter(m => current !== m.name).length})
          </summary>
          <div className="details-body">
            {externalModels
              .filter(m => current !== m.name)
              .map(m => (
                <div className="row" key={m.name}>
                  <span className="model-meta">
                    <span className="perm-name">{m.name}</span>
                    <small>via {m.runtime === 'Sumi' ? 'Beside' : m.runtime || 'Local'}</small>
                  </span>
                  <button
                    type="button"
                    className="btn-sm"
                    disabled={!!working}
                    onClick={() => void selectModel('model', m, m.name)}
                  >
                    {working === m.name ? 'Switching…' : 'Use'}
                  </button>
                </div>
              ))}
          </div>
        </details>
      )}

      {working === 'gguf' && (
        <div className="feedback-pill" role="status">
          Installing {downloading}…
          {choices
            .filter(m => m.label === downloading && m.bytes > 0)
            .map(m => (
              <progress
                key={m.label}
                aria-label="Model download"
                value={m.bytes}
                max={m.total}
              />
            ))}
        </div>
      )}

      {simple ? (
        <button type="button" className="bar-link" disabled={!!working} onClick={() => void selectModel('importModel', {}, 'model file')}>
          Choose a model file…
        </button>
      ) : (
        <details>
          <summary>Advanced · Import custom GGUF</summary>
          <div className="details-body">
            <div className="action-row">
              <button
                type="button"
                className="btn-sm"
                disabled={!!working}
                onClick={() =>
                  void selectModel('importModel', {}, 'custom model')
                }
              >
                Choose GGUF file…
              </button>
            </div>
          </div>
        </details>
      )}

      {notice && (
        <div className="feedback-pill" role="status">
          {notice}
        </div>
      )}
    </section>
  );
}
