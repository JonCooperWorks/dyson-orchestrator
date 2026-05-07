import React from 'react';
import { useApi } from '../hooks/useApi.jsx';
import { fmtTime, shortId } from '../utils/format.js';

export function SkillsPage() {
  const { client } = useApi();
  const [catalog, setCatalog] = React.useState(null);
  const [inventory, setInventory] = React.useState(null);
  const [err, setErr] = React.useState('');

  React.useEffect(() => {
    let cancelled = false;
    Promise.all([
      client.listMarketplaceSkills(),
      client.listSkills(),
    ]).then(([cat, inv]) => {
      if (cancelled) return;
      setCatalog(cat || { skills: [], errors: [], sources: [] });
      setInventory(Array.isArray(inv) ? inv : []);
      setErr('');
    }).catch(e => {
      if (!cancelled) setErr(e?.detail || e?.message || 'skills load failed');
    });
    return () => { cancelled = true; };
  }, [client]);

  const catalogSkills = catalog?.skills || [];
  const inventoryRows = inventory || [];
  const grouped = groupInventory(inventoryRows);

  return (
    <main className="detail-pane" style={{overflowY:'auto'}}>
      <div style={{maxWidth:1180, width:'100%', margin:'0 auto', padding:'22px'}}>
        <header style={{display:'flex', alignItems:'end', gap:16, marginBottom:18}}>
          <div>
            <div className="eyebrow">skills</div>
            <h1 style={{margin:'4px 0 0', fontSize:28}}>Marketplace and fleet inventory</h1>
          </div>
          <span style={{flex:1}}/>
          <span className="badge badge-info">{catalogSkills.length} catalog</span>
          <span className="badge badge-info">{inventoryRows.length} installed</span>
        </header>
        {err ? <div className="error">{err}</div> : null}

        <section className="panel">
          <div className="panel-title">marketplace catalog</div>
          {catalogSkills.length === 0 ? (
            <p className="muted small">no marketplace skills available</p>
          ) : (
            <div style={{display:'grid', gap:8}}>
              {catalogSkills.map(skill => (
                <SkillCatalogRow key={`${skill.marketplace_id}/${skill.name}`} skill={skill}/>
              ))}
            </div>
          )}
          {(catalog?.errors || []).length > 0 ? (
            <div style={{marginTop:14}}>
              <div className="eyebrow">catalog errors</div>
              {catalog.errors.map(e => (
                <div key={`${e.marketplace_id}:${e.error}`} className="error" style={{marginTop:6}}>
                  {e.marketplace_id}: {e.error}
                </div>
              ))}
            </div>
          ) : null}
        </section>

        <section className="panel">
          <div className="panel-title">installed across swarm</div>
          {grouped.length === 0 ? (
            <p className="muted small">no mirrored skills yet</p>
          ) : (
            <div style={{display:'grid', gap:8}}>
              {grouped.map(group => <SkillInventoryGroup key={group.skill} group={group}/>)}
            </div>
          )}
        </section>
      </div>
    </main>
  );
}

export function SkillCatalogRow({ skill }) {
  return (
    <div className="mcp-row" style={{alignItems:'start'}}>
      <div style={{minWidth:0, flex:1}}>
        <div style={{display:'flex', alignItems:'center', gap:8, flexWrap:'wrap'}}>
          <strong className="mono">{skill.name}</strong>
          <span className="badge">{skill.marketplace_id}</span>
          <span className="muted small">v{skill.version}</span>
          <span className="muted small">{skill.content_type}</span>
        </div>
        <div className="muted small" style={{marginTop:4}}>{skill.description}</div>
      </div>
    </div>
  );
}

export function SkillInventoryGroup({ group }) {
  return (
    <div className="mcp-row" style={{alignItems:'start'}}>
      <div style={{minWidth:0, flex:1}}>
        <div style={{display:'flex', alignItems:'center', gap:8, flexWrap:'wrap'}}>
          <strong className="mono">{group.skill}</strong>
          <span className="badge badge-info">{group.rows.length} instance{group.rows.length === 1 ? '' : 's'}</span>
          {group.originKinds.map(kind => <span key={kind} className="badge">{kind}</span>)}
        </div>
        <div className="muted small" style={{marginTop:4}}>{group.description || '—'}</div>
        <div style={{display:'flex', flexWrap:'wrap', gap:6, marginTop:8}}>
          {group.rows.map(row => (
            <a
              key={`${row.instance_id}/${row.skill}`}
              className="chip"
              href={`#/i/${encodeURIComponent(row.instance_id)}/skills`}
              title={`last swept ${fmtTime(row.synced_at)}`}
            >
              {shortId(row.instance_id)}
            </a>
          ))}
        </div>
      </div>
    </div>
  );
}

export function SkillInventoryList({ rows }) {
  if (!rows || rows.length === 0) {
    return <p className="muted small">no mirrored skills yet</p>;
  }
  return (
    <div style={{display:'grid', gap:8}}>
      {rows.map(row => (
        <div key={`${row.instance_id}/${row.skill}`} className="mcp-row" style={{alignItems:'start'}}>
          <div style={{minWidth:0, flex:1}}>
            <div style={{display:'flex', gap:8, flexWrap:'wrap', alignItems:'center'}}>
              <strong className="mono">{row.skill}</strong>
              <span className="badge">{row.origin_kind || 'unknown'}</span>
              {row.version ? <span className="muted small">v{row.version}</span> : null}
              {!row.has_metadata ? <span className="badge badge-warn">no metadata</span> : null}
              {!row.has_body ? <span className="badge badge-warn">missing body</span> : null}
            </div>
            <div className="muted small" style={{marginTop:4}}>{row.description || '—'}</div>
            <div className="mono muted small" style={{marginTop:6}}>
              swept {fmtTime(row.synced_at)} · {row.source_path}
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function groupInventory(rows) {
  const map = new Map();
  for (const row of rows || []) {
    const key = row.skill || '';
    if (!key) continue;
    if (!map.has(key)) map.set(key, { skill: key, rows: [], originKinds: new Set(), description: '' });
    const group = map.get(key);
    group.rows.push(row);
    if (row.origin_kind) group.originKinds.add(row.origin_kind);
    if (!group.description && row.description) group.description = row.description;
  }
  return [...map.values()].map(group => ({
    ...group,
    originKinds: [...group.originKinds].sort(),
  })).sort((a, b) => a.skill.localeCompare(b.skill));
}
