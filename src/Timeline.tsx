import { memo, useEffect, useMemo, useRef, useState } from 'react';
import './timeline.css';

type Runtime = 'codex' | 'opencode';
export type TimelineProject = { runtime?: Runtime; root: string; generation: number; sequence?: number };
export type TimelineEvent = {
  runtime: Runtime; project: string; generation: number; sequence: number;
  receivedAtMs?: number | null; clientTurnId?: string | null; message: unknown;
};
type Batch = { events: TimelineEvent[]; truncated: boolean };
type Source = TimelineProject & Batch & { error?: string };
export type TimelineHistory = { id: string; turns?: { id: string; status: string; error?: unknown; items: { id: string; type: string; [key: string]: unknown }[] }[] };
type Row = {
  key: string; runtime: Runtime; project: string; generation: number; thread: string; turn: string;
  method: string; status: string; raw: unknown; received?: number | null; sequence?: number;
  clientTurn?: boolean; history?: boolean; clientGenerated?: boolean; duration?: number; runtimeTime?: number;
};
type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
const record = (value: unknown): Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
const str = (value: unknown) => typeof value === 'string' ? value : '';
const time = (value: unknown): number | undefined => typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 8640000000000000 ? value : undefined;
export const timelineKey = (project: TimelineProject) => JSON.stringify([project.runtime ?? 'codex', project.root, project.generation]);

// Every refresh is a projection of the existing backend ring, never an accumulating cache.
export function normalizeEvents(project: TimelineProject, batch: Batch): TimelineEvent[] {
  const events = new Map<number, TimelineEvent>();
  for (const event of batch.events) {
    if (event.runtime === (project.runtime ?? 'codex') && event.project === project.root && event.generation === project.generation && Number.isSafeInteger(event.sequence) && event.sequence > 0 && !events.has(event.sequence)) events.set(event.sequence, event);
  }
  return [...events.values()].sort((a, b) => a.sequence - b.sequence);
}

// Preserve each stream's sequence even across a wall-clock adjustment. Independent
// streams are merged by receipt time, with a deterministic source tie-breaker.
export function orderEvents(streams: TimelineEvent[][]): TimelineEvent[] {
  const remaining = streams.map(events => [...events]);
  const ordered: TimelineEvent[] = [];
  while (remaining.some(events => events.length)) {
    const available = remaining.filter(events => events.length);
    available.sort((a, b) => (time(a[0].receivedAtMs) ?? Infinity) - (time(b[0].receivedAtMs) ?? Infinity) || timelineKey({runtime:a[0].runtime,root:a[0].project,generation:a[0].generation}).localeCompare(timelineKey({runtime:b[0].runtime,root:b[0].project,generation:b[0].generation})));
    ordered.push(available[0].shift()!);
  }
  return ordered;
}

export function eventRow(event: TimelineEvent): Row {
  const message = record(event.message), params = record(message.params);
  const item = record(params.item), turn = record(params.turn), update = record(params.update);
  const method = str(message.method) || 'Unknown event';
  return {
    key: JSON.stringify([event.runtime,event.project,event.generation,event.sequence]),
    runtime: event.runtime, project: event.project, generation: event.generation, sequence: event.sequence,
    received: time(event.receivedAtMs), raw: event.message,
    thread: str(params.threadId) || str(params.conversationId) || str(params.sessionId) || str(record(params.thread).id),
    turn: str(params.turnId) || str(turn.id) || event.clientTurnId || '', clientTurn: Boolean(event.clientTurnId),
    method: [method, str(item.type) || str(update.sessionUpdate)].filter(Boolean).join(' · '),
    status: str(item.status) || str(turn.status) || str(update.status) || (method === 'error' ? params.willRetry === true ? 'error · retrying' : 'error' : message.id !== undefined ? 'request reported' : 'reported'),
    duration: time(item.durationMs), runtimeTime: time(params.completedAtMs) ?? time(params.startedAtMs),
  };
}

export function historyRows(project: TimelineProject, history: TimelineHistory): Row[] {
  return (history.turns ?? []).flatMap(turn => [
    ...turn.items.map((item, index) => ({
      key: JSON.stringify(['history',timelineKey(project),history.id,turn.id,item.id,index]),
      runtime: project.runtime ?? 'codex', project: project.root, generation: project.generation,
      thread: history.id, turn: turn.id, method: item.type, status: str(item.status) || 'status not reported',
      raw: item, history: true, clientGenerated: item.clientGenerated === true, duration: time(item.durationMs), clientTurn: project.runtime === 'opencode',
    })),
    {key:JSON.stringify(['history-turn',timelineKey(project),history.id,turn.id]),runtime:project.runtime ?? 'codex',project:project.root,generation:project.generation,thread:history.id,turn:turn.id,method:'Turn snapshot',status:turn.status,raw:{id:turn.id,status:turn.status,error:turn.error},history:true,clientTurn:project.runtime==='opencode'},
  ]);
}

export function filterTimeline(rows: Row[], project: string, thread: string, turn: string): Row[] {
  return rows.filter(row => (!project || timelineKey({runtime:row.runtime,root:row.project,generation:row.generation}) === project) && (!thread || row.thread === thread) && (!turn || row.turn === turn));
}

const TimelineEntry = memo(function TimelineEntry({ row }: { row: Row }) {
  const [open, setOpen] = useState(false);
  return <li className="timeline-entry">
    <details onToggle={event => setOpen(event.currentTarget.open)}>
      <summary><span className="timeline-time">{row.history ? 'Native history' : row.received === undefined || row.received === null ? 'Receipt time unavailable' : <time dateTime={new Date(row.received).toISOString()} title={`Received at ${new Date(row.received).toISOString()}`}>{new Date(row.received).toISOString().slice(11,23)} UTC</time>}</span><strong>{row.method}</strong><span>{row.status}</span></summary>
      <div className="timeline-detail"><dl>
        <dt>Source</dt><dd>{row.history ? row.clientGenerated ? 'Client-submitted placeholder; not yet replaced by a runtime item. Event time unavailable.' : 'Runtime-native snapshot; not an event replay. Event time unavailable.' : `Raw reported event · received at client · sequence ${row.sequence}`}</dd>
        <dt>Runtime / project</dt><dd>{row.runtime === 'opencode' ? 'OpenCode' : 'Codex'} · {row.project} · generation {row.generation}</dd>
        <dt>Thread</dt><dd>{row.thread || 'Not reported'}</dd><dt>{row.clientTurn ? 'Client-correlated turn' : 'Turn'}</dt><dd>{row.turn || 'Not reported'}</dd>
        {row.runtimeTime !== undefined && <><dt>Runtime-reported lifecycle time</dt><dd>{new Date(row.runtimeTime).toISOString()}</dd></>}
        {row.duration !== undefined && <><dt>Runtime-reported duration</dt><dd>{row.duration} ms</dd></>}
      </dl>{open && <pre tabIndex={0} aria-label="Reported event detail">{JSON.stringify(row.raw,null,2) ?? 'No payload reported'}</pre>}</div>
    </details>
  </li>;
});

export function TimelineView({ sources, history, project, thread, turn }: { sources: Source[]; history?: Row[]; project: string; thread: string; turn: string }) {
  const rows = filterTimeline(orderEvents(sources.map(source => source.events)).map(eventRow),project,thread,turn);
  const older = filterTimeline(history ?? [],project,thread,turn);
  return <>
    <p className="timeline-note">Live events use client receipt time. Order within each server follows its sequence; cross-server receipt order is approximate. CorpsTex saves no timeline data.</p>
    {sources.map(source => <div key={timelineKey(source)} className="timeline-notices">
      {source.truncated && <p role="status">{source.runtime ?? 'codex'} · {source.root}: earlier events were evicted from the 256-event / 4 MiB ring. Live activity is incomplete.</p>}
      {source.error && <p role="alert">{source.runtime ?? 'codex'} · {source.root}: {source.error} Shown events may be stale.</p>}
    </div>)}
    {rows.length ? <ol className="timeline-events" aria-label="Live execution events">{rows.map(row => <TimelineEntry key={row.key} row={row}/>)}</ol> : <p className="timeline-note">No execution activity matches these filters.</p>}
    <details className="timeline-history"><summary>Native history · {older.length} snapshots</summary><p className="timeline-note">From the open conversation’s loaded runtime history, in native turn/item order. Client-submitted placeholders are identified in their details. Load earlier messages in the conversation to inspect more. Snapshots can overlap live events; their execution times are unknown.</p><ol className="timeline-events" aria-label="Native activity snapshots">{older.map(row => <TimelineEntry key={row.key} row={row}/>)}</ol></details>
  </>;
}

export const ExecutionTimeline = memo(function ExecutionTimeline({ projects, history, historyProject, invoke }: { projects: TimelineProject[]; history: TimelineHistory | null; historyProject: TimelineProject | null; invoke: Invoke }) {
  const [open,setOpen] = useState(false);
  const [sources,setSources] = useState<Source[]>([]);
  const [project,setProject] = useState('');
  const [thread,setThread] = useState('');
  const [turn,setTurn] = useState('');
  const latest = useRef(projects); latest.current = projects;
  const bindings = JSON.stringify(projects.map(timelineKey).sort());
  useEffect(() => {setProject('');setThread('');setTurn('');},[bindings]);
  useEffect(() => {
    if (!open) { setSources([]); return; }
    let disposed = false, busy = false;
    const previous = new Map<string, { sequence?: number; source: Source }>();
    setSources([]);
    async function refresh() {
      if (busy) return; busy = true;
      const next = await Promise.all(latest.current.map(async entry => {
        const key = timelineKey(entry), old = previous.get(key);
        if (old && entry.sequence !== undefined && old.sequence === entry.sequence && !old.source.error) return old.source;
        try {
          const batch = await invoke<Batch>('project_events',{runtime:entry.runtime ?? 'codex',root:entry.root,generation:entry.generation,after:0});
          const source = {...entry,events:normalizeEvents(entry,batch),truncated:batch.truncated};
          previous.set(key,{sequence:entry.sequence,source});
          return source;
        } catch {
          return {...entry,events:old?.source.events ?? [],truncated:old?.source.truncated ?? false,error:'Activity could not be read. Retry on the next refresh.'};
        }
      }));
      if (!disposed) setSources(next);
      busy = false;
    }
    void refresh(); const timer = window.setInterval(() => { void refresh(); },750);
    return () => {disposed = true; window.clearInterval(timer);};
  },[open,bindings,invoke]);
  const native = useMemo(() => open && historyProject && history ? historyRows(historyProject,history) : [],[open,historyProject?.runtime,historyProject?.root,historyProject?.generation,history]);
  const currentSources = sources.filter(source => projects.some(entry => timelineKey(entry) === timelineKey(source)));
  const rows = [...currentSources.flatMap(source => source.events.map(eventRow)),...native];
  const projectRows = filterTimeline(rows,project,'','');
  const threads = [...new Set(projectRows.map(row => row.thread).filter(Boolean))];
  const turns = [...new Set(filterTimeline(projectRows,'',thread,'').map(row => row.turn).filter(Boolean))];
  return <section className="timeline" aria-labelledby="timeline-title"><details onToggle={event => setOpen(event.currentTarget.open)}>
    <summary id="timeline-title" className="panel-heading">Execution timeline</summary>
    {open ? <div className="timeline-body">
      <div className="timeline-filters">
        <label>Project<select aria-label="Timeline project" value={project} onChange={event=>{setProject(event.target.value);setThread('');setTurn('');}}><option value="">All projects and runtimes</option>{projects.map(entry=><option key={timelineKey(entry)} value={timelineKey(entry)}>{entry.runtime ?? 'codex'} · {entry.root} · {entry.generation}</option>)}</select></label>
        <label>Thread<select aria-label="Timeline thread" value={thread} onChange={event=>{setThread(event.target.value);setTurn('');}}><option value="">All threads / unassigned</option>{[...new Set([...threads,...(thread?[thread]:[])])].map(id=><option key={id}>{id}</option>)}</select></label>
        <label>Turn<select aria-label="Timeline turn" value={turn} onChange={event=>setTurn(event.target.value)}><option value="">All turns / unassigned</option>{[...new Set([...turns,...(turn?[turn]:[])])].map(id=><option key={id}>{id}</option>)}</select></label>
      </div>
      <TimelineView sources={currentSources} history={native} project={project} thread={thread} turn={turn}/>
    </div> : <span className="timeline-closed-note">{projects.length ? 'Live activity and native history' : 'No execution activity'}</span>}
  </details></section>;
});
