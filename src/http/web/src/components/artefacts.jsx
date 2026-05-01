/* swarm — Artefacts pages (cached artefacts surface).
 *
 * Two routes share one component:
 *
 *   #/artefacts                    → all my artefacts (cross-instance)
 *   #/i/<id>/artefacts             → per-instance
 *
 * Both render the same `<ArtefactsView>` shell — back link, title +
 * subtitle, and a paginated table.  The cross-instance variant adds
 * an "instance" column; the per-instance variant adds a single
 * "sweep" button in the panel header that prompts for a chat id and
 * walks the cube's listing into the cache.
 *
 * Bytes live on swarm under [backup].local_cache_dir/artefacts/.
 * Each row exposes:
 *   - "open"  — authenticated fetch + blob URL window.open
 *   - "share" — quick-mint a 7d anonymous share link
 *   - "drop"  — remove the swarm cache copy
 */

import React from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkBreaks from 'remark-breaks';
import { useApi } from '../hooks/useApi.jsx';

const QUICK_TTL = '7d';
const PAGE_SIZE = 25;
const MD_PLUGINS = [remarkGfm, remarkBreaks];

export function MyArtefactsPage() {
  const { client } = useApi();
  const load = React.useCallback(
    () => client.listMyArtefacts({ limit: 1000 }),
    [client],
  );
  return (
    <ArtefactsView
      backHref="#/"
      subtitle="Everything your agents have produced and that has reached swarm.  Stored on swarm, so they survive cube reset.  Pick any to share via an anonymous link."
      load={load}
      showInstance
    />
  );
}

export function InstanceArtefactsPage({ instanceId }) {
  const { client } = useApi();
  const load = React.useCallback(
    () => client.listInstanceArtefacts(instanceId),
    [client, instanceId],
  );
  const sweep = React.useCallback(async () => {
    const chatId = prompt('chat id to sweep cube → cache:');
    if (!chatId || !chatId.trim()) return null;
    await client.sweepInstanceArtefacts(instanceId, chatId.trim());
    return null;
  }, [client, instanceId]);
  return (
    <ArtefactsView
      backHref={`#/i/${encodeURIComponent(instanceId)}`}
      subtitle="Cached artefacts for this instance.  Reads come from swarm first; cube is only hit on cache miss.  Survives reset."
      load={load}
      onSweep={sweep}
    />
  );
}

/// Shared shell.  `load` is the source of rows (refreshable); `onSweep`
/// is the optional cube→cache button shown only on the per-instance
/// variant.  Pagination is client-side over the loaded rows; the
/// underlying list endpoints already cap at 1000 (cross-instance) /
/// per-instance listings are short by construction.
function ArtefactsView({ backHref, subtitle, load, onSweep, showInstance }) {
  const { client } = useApi();
  const [rows, setRows] = React.useState(null);
  const [err, setErr] = React.useState(null);
  const [busy, setBusy] = React.useState(false);
  const [minted, setMinted] = React.useState(null);
  const [page, setPage] = React.useState(1);
  const [opened, setOpened] = React.useState(null); // row currently in the reader

  const refresh = React.useCallback(async () => {
    setErr(null);
    try {
      const list = await load();
      setRows(Array.isArray(list) ? list : []);
    } catch (e) {
      setErr(e?.detail || e?.message || 'list failed');
    }
  }, [load]);

  React.useEffect(() => { refresh(); }, [refresh]);
  // Reset to page 1 whenever the row set changes (refresh, sweep).
  React.useEffect(() => { setPage(1); }, [rows && rows.length]);

  const sweepClick = onSweep
    ? async () => {
        setBusy(true); setErr(null);
        try {
          await onSweep();
          await refresh();
        } catch (e) {
          setErr(e?.detail || e?.message || 'sweep failed');
        } finally {
          setBusy(false);
        }
      }
    : null;

  return (
    <main className="page page-edit">
      <header className="page-header">
        <a className="btn btn-ghost btn-sm" href={backHref}>← back</a>
        <h1 className="page-title">artefacts</h1>
        <p className="page-sub muted">{subtitle}</p>
      </header>

      {err ? <div className="error">{err}</div> : null}
      {minted ? <MintedBanner minted={minted} onDismiss={() => setMinted(null)}/> : null}

      <ArtefactTable
        rows={rows}
        page={page}
        setPage={setPage}
        client={client}
        busy={busy}
        setBusy={setBusy}
        setErr={setErr}
        setMinted={setMinted}
        refresh={refresh}
        showInstance={showInstance}
        sweepClick={sweepClick}
        onOpen={setOpened}
      />

      {opened ? (
        <ArtefactReader
          row={opened}
          client={client}
          onClose={() => setOpened(null)}
        />
      ) : null}
    </main>
  );
}

function ArtefactTable({
  rows, page, setPage, client, busy, setBusy, setErr, setMinted, refresh,
  showInstance, sweepClick, onOpen,
}) {
  if (rows === null) return <p className="muted small">loading…</p>;
  if (rows.length === 0) {
    return (
      <p className="muted small">
        no cached artefacts yet — they appear here as soon as one is read through swarm
        (e.g. via an existing share link or via <em>sweep</em> on a per-instance view).
      </p>
    );
  }

  const total = rows.length;
  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const safePage = Math.min(Math.max(1, page), pageCount);
  const start = (safePage - 1) * PAGE_SIZE;
  const visible = rows.slice(start, start + PAGE_SIZE);

  const remove = async (row) => {
    if (!confirm(`Remove cached copy of "${row.title}"?  This drops the swarm row + on-disk body; the live cube still has it (until reset).`)) return;
    setBusy(true); setErr(null);
    try {
      await client.deleteInstanceArtefact(row.instance_id, row.id);
      await refresh();
    } catch (e) {
      setErr(e?.detail || e?.message || 'delete failed');
    } finally {
      setBusy(false);
    }
  };

  const share = async (row) => {
    setBusy(true); setErr(null);
    try {
      const m = await client.mintShare(row.instance_id, row.id, {
        chat_id: row.chat_id,
        ttl: QUICK_TTL,
        label: null,
      });
      setMinted(m);
    } catch (e) {
      setErr(e?.detail || e?.message || 'share mint failed');
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="panel">
      <div className="panel-header">
        <div className="panel-title">artefacts</div>
        <div className="panel-actions">
          {sweepClick ? (
            <button
              className="btn btn-ghost btn-sm"
              onClick={sweepClick}
              disabled={busy}
              title="sweep a chat's artefacts from cube into the swarm cache"
            >{busy ? 'sweeping…' : 'sweep'}</button>
          ) : null}
          <button className="btn btn-ghost btn-sm" onClick={refresh} title="refresh">↻</button>
        </div>
      </div>
      <table className="rows">
        <thead><tr>
          <th>title</th>
          <th>kind</th>
          {showInstance ? <th>instance</th> : null}
          <th>chat</th>
          <th>size</th>
          <th>cached</th>
          <th></th>
        </tr></thead>
        <tbody>
          {visible.map(r => (
            <tr key={`${r.instance_id}/${r.id}`}>
              <td data-label="title">
                <span title={r.id}>{r.title || r.id}</span>
              </td>
              <td data-label="kind"><span className="badge badge-faint">{r.kind}</span></td>
              {showInstance ? (
                <td data-label="instance">
                  <a className="mono-sm" href={`#/i/${encodeURIComponent(r.instance_id)}/artefacts`}>
                    {shortId(r.instance_id)}
                  </a>
                </td>
              ) : null}
              <td data-label="chat" className="mono-sm muted">{shortId(r.chat_id)}</td>
              <td data-label="size" className="muted small">{fmtBytes(r.bytes)}</td>
              <td data-label="cached" className="muted small">{fmtTime(r.cached_at)}</td>
              <td className="row-actions">
                <button
                  className="btn btn-ghost btn-sm"
                  onClick={() => onOpen && onOpen(r)}
                  title="open the cached body inline"
                >open</button>
                <button
                  className="btn btn-ghost btn-sm"
                  onClick={() => share(r)}
                  disabled={busy}
                  title={`mint a ${QUICK_TTL} anonymous share link`}
                >share</button>
                <button
                  className="btn btn-ghost btn-sm"
                  onClick={() => remove(r)}
                  disabled={busy}
                  title="remove the swarm cache copy"
                >drop</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {pageCount > 1 ? (
        <Pagination
          page={safePage}
          pageCount={pageCount}
          total={total}
          start={start}
          shown={visible.length}
          onPage={setPage}
        />
      ) : null}
    </section>
  );
}

function Pagination({ page, pageCount, total, start, shown, onPage }) {
  return (
    <div
      className="panel-footer muted small"
      style={{ display: 'flex', gap: 8, alignItems: 'center', justifyContent: 'space-between', padding: '8px 12px' }}
    >
      <span>
        showing {start + 1}–{start + shown} of {total}
      </span>
      <span style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
        <button
          className="btn btn-ghost btn-sm"
          onClick={() => onPage(page - 1)}
          disabled={page <= 1}
          title="previous page"
        >‹ prev</button>
        <span style={{ minWidth: 70, textAlign: 'center' }}>{page} / {pageCount}</span>
        <button
          className="btn btn-ghost btn-sm"
          onClick={() => onPage(page + 1)}
          disabled={page >= pageCount}
          title="next page"
        >next ›</button>
      </span>
    </div>
  );
}

/// In-page reader.  Fires an authenticated bytes fetch and renders
/// the result based on (kind, mime, name):
///   - image/* (or kind=='image') → <img> from a blob URL
///   - text/markdown (or .md/.markdown name) → react-markdown w/ gfm + breaks
///   - text/* / json / xml → <pre> raw text
///   - everything else → download card
/// Shape mirrors dyson's `ArtefactReader` (views-secondary.jsx) so the
/// two SPAs feel the same on click.  Modal-style overlay; Esc + a
/// scrim click both close.  Blob URLs are revoked on close to keep the
/// browser's memory bounded.
function ArtefactReader({ row, client, onClose }) {
  const [state, setState] = React.useState({
    loading: true, err: null, mime: '', text: null, blob: null, blobUrl: null,
  });

  React.useEffect(() => {
    let cancelled = false;
    let createdUrl = null;
    (async () => {
      try {
        const { blob, mime, text } = await client.fetchInstanceArtefactBytes(
          row.instance_id, row.id,
        );
        if (cancelled) return;
        // Image branch needs an objectURL; markdown only needs the
        // text.  Other types may want both (preview download card).
        const isImage = (mime || '').startsWith('image/')
          || row.kind === 'image'
          || (row.mime || '').startsWith('image/');
        if (isImage) {
          createdUrl = URL.createObjectURL(blob);
        }
        setState({ loading: false, err: null, mime, text, blob, blobUrl: createdUrl });
      } catch (e) {
        if (cancelled) return;
        setState({ loading: false, err: String(e?.message || e), mime: '', text: null, blob: null, blobUrl: null });
      }
    })();
    return () => {
      cancelled = true;
      if (createdUrl) URL.revokeObjectURL(createdUrl);
    };
  }, [client, row.instance_id, row.id]);

  // Esc closes — same posture as a confirm modal.
  React.useEffect(() => {
    const onKey = (e) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const download = () => {
    if (!state.blob) return;
    const url = state.blobUrl || URL.createObjectURL(state.blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = row.title || row.id;
    document.body.appendChild(a); a.click(); a.remove();
    if (!state.blobUrl) setTimeout(() => URL.revokeObjectURL(url), 5000);
  };

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.7)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        zIndex: 1000, padding: 20,
      }}
    >
      <section
        onClick={(e) => e.stopPropagation()}
        className="panel"
        style={{
          maxWidth: '900px', width: '100%', maxHeight: '90vh',
          display: 'flex', flexDirection: 'column', overflow: 'hidden',
        }}
      >
        <div className="panel-header">
          <div className="panel-title" style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {row.title || row.id}
          </div>
          <div className="panel-actions">
            <span className="muted small" style={{ marginRight: 8 }}>
              {row.kind} · {fmtBytes(row.bytes)}
            </span>
            <button
              className="btn btn-ghost btn-sm"
              onClick={download}
              disabled={!state.blob}
              title="download the bytes"
            >download</button>
            <button className="btn btn-ghost btn-sm" onClick={onClose} title="close (esc)">×</button>
          </div>
        </div>
        <div style={{ overflow: 'auto', padding: 16, flex: 1 }}>
          <ArtefactBody row={row} state={state} />
        </div>
      </section>
    </div>
  );
}

function ArtefactBody({ row, state }) {
  if (state.loading) return <p className="muted small">loading…</p>;
  if (state.err) return <div className="error">{state.err}</div>;

  const mime = state.mime || row.mime || '';
  const title = row.title || '';
  const isImage = mime.startsWith('image/') || row.kind === 'image';
  const isMarkdown = mime === 'text/markdown'
    || /\.(md|markdown)$/i.test(title)
    || (row.kind === 'security_review' && state.text);
  const isPlainText = !isImage && !isMarkdown
    && (mime.startsWith('text/') || /json|xml/.test(mime))
    && state.text != null;

  if (isImage && state.blobUrl) {
    return (
      <img
        src={state.blobUrl}
        alt={title}
        style={{ maxWidth: '100%', display: 'block', margin: '0 auto' }}
      />
    );
  }
  if (isMarkdown && state.text != null) {
    return (
      <div className="md-body">
        <ReactMarkdown remarkPlugins={MD_PLUGINS}>{state.text}</ReactMarkdown>
      </div>
    );
  }
  if (isPlainText) {
    return (
      <pre
        className="mono-sm"
        style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word', margin: 0 }}
      >{state.text}</pre>
    );
  }
  // Binary fallback — render a download card.  The header's "download"
  // button is the actionable surface; this is just the inline notice.
  return (
    <div className="muted small">
      Binary artefact ({mime || 'unknown type'}) — use download.
    </div>
  );
}

function MintedBanner({ minted, onDismiss }) {
  const [copied, setCopied] = React.useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(minted.url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch { /* ignore */ }
  };
  return (
    <div className="banner banner-info">
      <div>
        share minted — capability is in the URL, copy it now (revoke from
        the per-instance shares panel anytime):
      </div>
      <code className="mono-sm" style={{ display: 'block', marginTop: 4, wordBreak: 'break-all' }}>
        {minted.url}
      </code>
      <div className="muted small" style={{ marginTop: 6 }}>
        expires {fmtTime(minted.expires_at)}.
      </div>
      <div style={{ marginTop: 10, display: 'flex', gap: 8 }}>
        <button className="btn btn-sm btn-primary" onClick={copy}>{copied ? 'copied' : 'copy link'}</button>
        <button className="btn btn-ghost btn-sm" onClick={onDismiss}>dismiss</button>
      </div>
    </div>
  );
}

function shortId(s) {
  if (!s) return '—';
  return s.length > 12 ? `${s.slice(0, 8)}…${s.slice(-3)}` : s;
}

function fmtBytes(n) {
  if (!Number.isFinite(n) || n <= 0) return '—';
  const units = ['B', 'KB', 'MB', 'GB'];
  let i = 0; let v = n;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i += 1; }
  return `${v.toFixed(v < 10 && i > 0 ? 1 : 0)} ${units[i]}`;
}

function fmtTime(secs) {
  if (!secs) return '—';
  try { return new Date(secs * 1000).toISOString().replace('T', ' ').replace(/\.\d+Z$/, 'Z'); }
  catch { return String(secs); }
}
