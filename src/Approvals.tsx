import { useEffect, useRef, useState, useId } from 'react';
import { createPortal } from 'react-dom';
import stylesheet from './approvals.css?url';
import { QuestionForm, type Question } from './QuestionForm';

export type Approval = {
  runtime?:'codex'|'opencode'; root: string; generation: number; ticket: number; key: string;
  status: string; decisions: string[];
  request: { method: string; params: Record<string, unknown> };
  item: unknown;
};
type Decision = {type:'select';optionId:string} | { type: 'accept' | 'decline' | 'cancel' } | { type: 'answers'; answers: Record<string, string> };
type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
const identity = (a: Approval) => JSON.stringify([a.runtime ?? 'codex',a.root,a.generation,a.ticket]);
const text = (value: unknown) => typeof value === 'string' ? value : JSON.stringify(value, null, 2);

// Codex 0.166.0 server requests that expect a result only this client could
// truthfully produce. The protocol offers no decline answer for them; CorpsTex
// never fabricates tool output, credentials or attestation tokens. Each request
// stays visible until the runtime resolves it or the connection ends.
const clientBound: Record<string, { title: string; why: string }> = {
  'item/tool/call': { title: 'Client tool call', why: 'CorpsTex does not host client-executed tools and will not fabricate a tool result.' },
  'account/chatgptAuthTokens/refresh': { title: 'Credential refresh', why: 'CorpsTex does not manage Codex credentials and will not return access tokens.' },
  'attestation/generate': { title: 'Attestation request', why: 'CorpsTex has no attestation provider and will not fabricate a token.' },
};

export function ApprovalCard({ approval: a, busy, onReply }: { approval: Approval; busy: boolean; onReply: (decision: Decision) => void }) {
  const titleId = useId();
  const p = a.request.params;
  const questions = Array.isArray(p.questions) ? p.questions as Question[] : [];
  const client = clientBound[a.request.method];
  const blocked = busy || a.status !== 'ready';
  return <article aria-labelledby={titleId}>
    <h2 id={titleId}>{a.request.method==='session/request_permission' ? 'OpenCode permission request' : client ? `${client.title} — CorpsTex cannot answer` : a.request.method.includes('permissions') ? 'Permission request' : a.request.method.includes('fileChange') || a.request.method === 'applyPatchApproval' ? 'File change approval' : a.request.method.includes('requestUserInput') ? 'Input requested' : a.request.method.includes('elicitation') ? 'MCP request' : a.request.method.includes('commandExecution') || a.request.method === 'execCommandApproval' ? p.networkApprovalContext ? 'Network access approval' : p.kind === 'writeStdin' ? 'Terminal input approval' : 'Command approval' : 'Unsupported server request'}</h2>
    <details className="request-context" open={!a.decisions.includes('answers') || undefined}>
    <summary>Request context · {a.root}</summary><dl>
      <dt>Runtime</dt><dd>{a.runtime==='opencode'?'OpenCode':'Codex'}</dd>
      <dt>Project</dt><dd>{a.root}</dd>
      <dt>Server generation</dt><dd>{a.generation}</dd>
      <dt>Thread</dt><dd>{text(p.threadId ?? p.conversationId ?? p.sessionId) ?? 'Not reported'}</dd>
      <dt>Turn</dt><dd>{text(p.turnId) ?? 'Not reported'}</dd>
      <dt>Request ID</dt><dd>{a.key}</dd>
      <dt>Item / callback</dt><dd>{text(p.itemId ?? p.callId) ?? 'Not reported'}{p.approvalId ? ` / ${text(p.approvalId)}` : ''}</dd>
    </dl></details>
    {a.status !== 'ready' && <p role="alert">This server is {a.status}. This request cannot be answered. A response outcome may be unknown; CorpsTex will not retry a decision. Review reconnect or exit to clear the lost connection.</p>}
    {!a.decisions.includes('answers') && <p>Review the reported command, access, and scope before deciding. The originating runtime controls permissions.</p>}
    {p.reason != null && <p><strong>Reason: </strong>{text(p.reason)}</p>}
    {p.cwd != null && <p><strong>Working directory: </strong>{text(p.cwd)}</p>}
    {p.command != null && <pre aria-label="Requested command">{text(p.command)}</pre>}
    {p.networkApprovalContext != null && <><h3>Requested network destination</h3><pre>{text(p.networkApprovalContext)}</pre></>}
    {p.permissions != null && <><h3>Requested permissions · current turn</h3><pre>{text(p.permissions)}</pre></>}
    {p.grantRoot != null && <p><strong>Requested write root: </strong>{text(p.grantRoot)}. The server may apply this to the remainder of the session.</p>}
    {(a.request.method.includes('fileChange') || a.request.method === 'applyPatchApproval') && <><h3>Proposed file changes</h3>{a.item || p.fileChanges ? <pre>{text(a.item ?? p.fileChanges)}</pre> : <p>Codex did not provide file details in the available state. Review the full request before deciding.</p>}</>}
    {a.request.method === 'mcpServer/elicitation/request' && <p>MCP forms and URL flows are not supported yet. You can explicitly decline or cancel this request.</p>}
    <details><summary>Full reported request and permission context</summary><pre>{text(a.request)}</pre></details>
    {a.decisions.length === 0 && (client
      ? <p role="alert">The official protocol expects a result only this client could truthfully produce: {client.why} No decision has been sent, and the request stays listed until the runtime resolves it or the connection ends. Stop the turn from the conversation composer to unblock active work.</p>
      : <p role="alert">No supported response is available for this request. No decision has been sent. Stop the turn or review the connection.</p>)}
    {a.decisions.includes('answers') && <QuestionForm questions={questions} blocked={blocked} onSubmit={answers => onReply({ type: 'answers', answers })} />}
    <div className="actions">
      {a.decisions.includes('options') && Array.isArray(p.options) && (p.options as {optionId:string;name:string;kind:string}[]).map(option=><button type="button" key={option.optionId} className={option.kind.startsWith('allow')?'allow':undefined} disabled={blocked} onClick={()=>onReply({type:'select',optionId:option.optionId})}>{option.name}{option.kind.endsWith('always')?' · persists in OpenCode':''}</button>)}
      {a.decisions.includes('decline') && <button type="button" disabled={blocked} onClick={() => onReply({ type: 'decline' })}>{p.permissions ? 'Deny requested access' : 'Decline'}</button>}
      {a.decisions.includes('cancel') && <button type="button" disabled={blocked} onClick={() => onReply({ type: 'cancel' })}>{a.runtime==='opencode' || a.request.method === 'mcpServer/elicitation/request' ? 'Cancel request' : 'Cancel and interrupt turn'}</button>}
      {a.decisions.includes('accept') && <button className="allow" type="button" disabled={blocked} onClick={() => onReply({ type: 'accept' })}>{p.permissions ? 'Grant requested access for this turn' : a.request.method.includes('commandExecution') ? 'Allow once' : 'Approve requested change'}</button>}
    </div>
  </article>;
}

function QueueDialog({ approvals, onClose, invoke, errorText, busyKeys }: { approvals: Approval[]; onClose: () => void; invoke: Invoke; errorText: (error: unknown) => string; busyKeys:Set<string> }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [selected, setSelected] = useState('');
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState('');
  const current = approvals.find(a => identity(a) === selected);
  useEffect(() => { const d = dialog.current; d?.showModal(); return () => d?.close(); }, []);
  useEffect(() => {
    if (selected && !current) dialog.current?.querySelector<HTMLButtonElement>('header button')?.focus();
  }, [selected, current]);
  async function reply(decision: Decision) {
    if (!current || busy) return;
    setBusy(true); setFeedback('');
    try {
      await invoke('approval_reply', { runtime:current.runtime, root: current.root, generation: current.generation, reply: { key: current.key, ticket: current.ticket, decision } });
      setSelected(''); setFeedback('Response sent to the originating runtime. Select another request to review it.');
      dialog.current?.querySelector<HTMLButtonElement>('header button')?.focus();
    } catch (error) { setFeedback(errorText(error)); }
    finally { setBusy(false); }
  }
  return <dialog ref={dialog} aria-labelledby="approvals-title" onCancel={event => { event.preventDefault(); onClose(); }} onClose={event => { if (!event.currentTarget.open) onClose(); }}>
    <header><div><p className="eyebrow">CORPSTEX · TRUSTED CONTROLS</p><h1 id="approvals-title">Approval center</h1></div><button type="button" autoFocus onClick={onClose}>Close</button></header>
    <p className="shortcut-hint">Escape to close</p>
    <p>All active projects · {approvals.length} outstanding requests. Closing this panel sends no response.</p>
    <p role="status">{feedback}</p>
    <nav aria-label="Outstanding requests">{approvals.map(a => <button type="button" key={identity(a)} disabled={busy} aria-pressed={identity(a) === selected} onClick={() => { setSelected(identity(a)); setFeedback(''); }}>{a.root} · {a.request.method} · {a.key}{clientBound[a.request.method] ? ' · cannot answer' : ''}{a.status !== 'ready' ? ` · ${a.status}` : ''}</button>)}</nav>
    {current ? <ApprovalCard key={identity(current)} approval={current} busy={busy || busyKeys.has(identity(current))} onReply={decision => { void reply(decision); }} /> : <p>{approvals.length ? selected ? 'This request was resolved or cleared by its runtime. Select a request to review it.' : 'Select a request to review its context and decisions.' : 'No outstanding requests.'}</p>}
  </dialog>;
}

// Theme CSS is confined to the ordinary shell; trusted controls live in a closed
// shadow root with a separate stylesheet and fixed accessible colors.
export function ApprovalCenter({ enabled = true, approvals, exiting, invoke, errorText, busyKeys = new Set<string>() }: { enabled?:boolean; approvals: Approval[]; exiting: boolean; invoke: Invoke; errorText: (error: unknown) => string; busyKeys?:Set<string> }) {
  const host = useRef<HTMLDivElement>(null);
  const [shadow, setShadow] = useState<ShadowRoot | null>(null);
  const shadowRef = useRef<ShadowRoot | null>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (host.current) {
      shadowRef.current ??= host.current.attachShadow({ mode: 'closed' });
      setShadow(shadowRef.current);
    }
  }, []);
  useEffect(() => { if (exiting || !enabled) setOpen(false); }, [exiting, enabled]);
  return <>
    {enabled && !shadow && <button type="button" disabled>Approvals · {approvals.length}</button>}
    <div ref={host} data-approval-controls />
    {shadow && createPortal(<><link rel="stylesheet" href={stylesheet} />
      {enabled && <button className="launcher" data-pending={approvals.some(a => a.status === 'ready') && !exiting} type="button" aria-expanded={open && !exiting} title="Approval center" disabled={exiting} onClick={() => setOpen(true)}>Approvals · {approvals.length}</button>}
      <span className="sr-only" role="status">{approvals.filter(a => a.status === 'ready').length} requests awaiting review across all projects</span>
      {enabled && open && !exiting && <QueueDialog busyKeys={busyKeys} approvals={approvals} onClose={() => setOpen(false)} invoke={invoke} errorText={errorText} />}
    </>, shadow)}
  </>;
}


export function matchingApprovals(approvals: Approval[], project: {runtime?:'codex'|'opencode';root:string;generation:number} | null, thread: string | null) {
  if (!project || !thread) return [];
  return approvals.filter(a=>(a.runtime??'codex')===(project.runtime??'codex') && a.root===project.root && a.generation===project.generation && (a.request.params.threadId ?? a.request.params.conversationId ?? a.request.params.sessionId)===thread);
}
export function InlineApprovals({approvals,exiting,busyKeys,invoke,errorText,visible,onReveal}: {approvals:Approval[];exiting:boolean;busyKeys:Set<string>;invoke:Invoke;errorText:(error:unknown)=>string;visible:boolean;onReveal:()=>void}) {
  const host=useRef<HTMLDivElement>(null);
  const root=useRef<ShadowRoot | null>(null);
  const [shadow,setShadow]=useState<ShadowRoot | null>(null);
  const [feedback,setFeedback]=useState('');
  const [styled,setStyled]=useState(false);
  const seen=useRef(new Set<string>());
  const keys=JSON.stringify(approvals.map(identity));
  const reveal=useRef(onReveal);
  reveal.current=onReveal;
  useEffect(()=>{if(host.current){root.current??=host.current.attachShadow({mode:'closed'});setShadow(root.current);}},[]);
  useEffect(()=>{
    const viewport=host.current?.closest<HTMLElement>('.messages');
    if(!viewport||!shadow)return;
    const size=()=>host.current?.style.setProperty('--approval-viewport',`${Math.max(1,viewport.clientHeight-24)}px`);
    size();
    const observer=new ResizeObserver(size);observer.observe(viewport);
    return ()=>observer.disconnect();
  },[shadow]);
  useEffect(()=>{
    if(!shadow||!styled)return;
    const current=JSON.parse(keys) as string[];
    const fresh=current.filter(key=>!seen.current.has(key));
    // Retain only outstanding identities; hidden tabs consume arrivals without
    // navigating or jumping later when the owner returns to them.
    seen.current=new Set(current);
    if(!visible||exiting||!fresh.length)return;
    const viewport=host.current?.closest<HTMLElement>('.messages');
    const card=shadow.querySelectorAll<HTMLElement>('article')[current.indexOf(fresh.at(-1)!)];
    if(!viewport||!card)return;
    reveal.current();
    // Scroll only this transcript, never the page or another session. Instant
    // positioning also respects reduced motion and cancels an ongoing follow.
    const frame=requestAnimationFrame(()=>{
      host.current?.style.setProperty('--approval-viewport',`${Math.max(1,viewport.clientHeight-24)}px`);
      viewport.scrollTo({top:viewport.scrollTop+card.getBoundingClientRect().top-viewport.getBoundingClientRect().top-12,behavior:'instant'});
    });
    return ()=>cancelAnimationFrame(frame);
  },[keys,shadow,styled,visible,exiting]);
  async function reply(approval:Approval,decision:Decision) {
    if(exiting||busyKeys.has(identity(approval)))return;
    setFeedback('');
    try {await invoke('approval_reply',{runtime:approval.runtime,root:approval.root,generation:approval.generation,reply:{key:approval.key,ticket:approval.ticket,decision}});setFeedback('Response sent to the originating runtime.');}
    catch(error){setFeedback(errorText(error));}
  }
  return <div className="inline-approvals" ref={host} hidden={!approvals.length}>
    {!shadow&&<p>Approval requested. Open the Approval center to respond.</p>}
    {shadow&&createPortal(<><link rel="stylesheet" href={stylesheet} onLoad={()=>setStyled(true)}/><section className="inline-queue" aria-label="Approvals for this conversation"><p className="eyebrow">CORPSTEX · TRUSTED CONTROLS</p><h2>Your response is needed</h2><p>These requests are also available in the Approval center.</p>{approvals.map(a=><ApprovalCard key={identity(a)} approval={a} busy={exiting||busyKeys.has(identity(a))} onReply={decision=>void reply(a,decision)}/>)}{feedback&&<p role="status">{feedback}</p>}</section></>,shadow)}
  </div>;
}
