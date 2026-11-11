// Transient, allowlisted client metadata only. Never retain raw errors or RPC payloads.
type Runtime = 'codex' | 'opencode' | 'unknown';
type State = 'starting' | 'ready' | 'failed' | 'stopped' | 'unknown';
type Context = {runtime:Runtime;project:string;generation:number|null;state:State;activeTurns:number;pendingRequests:number};
type Event = Context & {at:string;operation:string;kind:string};
export type Diagnostic = Event & {id:number;firstSeen:string;lastSeen:string;count:number;significant:boolean;dismissed:boolean;code:number|null;related:Event[]};
type Snapshot = {entries:Diagnostic[];omitted:number};
// Monaco 0.56 uses this exact Error signature for normal cancellation, including
// its WebKit clipboard workaround when a later gesture supersedes a write.
export function isEditorCancellation(error:unknown):boolean {
  return error instanceof Error && error.name==='Canceled' && error.message==='Canceled';
}
const commands = new Set(['project_view','connect_project','conversation_request','conversation_view','project_events','approval_reply','default_project_root','cancel_exit','confirm_exit','editor_request','editor_lsp_request','editor_install','editor_install_plan','editor_dirty','editor_exit_pending','reconnect_opencode','frontend']);
const actions = new Set(['send','steer','interrupt','resume','newThread','listThreads','listAllThreads','listArchivedThreads','listModels','listSkills','readSettings','readRequirements','writeSettings','listMcpStatus','fork','archive','unarchive','rename','compact','review']);
const significantKinds = new Set(['missingExecutable','openCodeUnavailable','unsupportedVersion','spawn','transport','malformedMessage','messageTooLarge','unsupportedProtocol','serverExit','timeout','bridge','frontendError']);
const knownKinds = new Set([...significantKinds,'invalidRequest','openCodeUnsupported','openCodeAuthRequired','invalidProject','notSubscribed','staleGeneration','busy','exitPending','confirmationRequired','cancelled','remote','conflict','encoding','size','permission','unknown']);
const obj = (v:unknown):Record<string,unknown> => v!==null&&typeof v==='object'&&!Array.isArray(v)?v as Record<string,unknown>:{};
const count = (v:unknown) => typeof v==='number'&&Number.isSafeInteger(v)&&v>=0?Math.min(v,999999):0;
const generation = (v:unknown) => typeof v==='number'&&Number.isSafeInteger(v)&&v>=0?v:null;
const state = (v:unknown):State => ['starting','ready','failed','stopped'].includes(String(v))?v as State:'unknown';
const runtime = (v:unknown):Runtime => v==='codex'||v==='opencode'?v:'unknown';
export function diagnosticSummary(entry:Pick<Diagnostic,'kind'|'operation'>):string {
  if(entry.kind==='frontendError')return 'The application encountered an unexpected frontend failure.';
  if(entry.operation==='project_view')return 'Project connection status could not be checked.';
  if(entry.kind==='serverExit')return 'A project runtime connection failed.';
  if(entry.kind==='messageTooLarge')return 'A runtime message exceeded the client transport limit.';
  if(entry.kind==='missingExecutable'||entry.kind==='openCodeUnavailable')return 'The selected runtime executable could not be found.';
  if(entry.kind==='malformedMessage'||entry.kind==='unsupportedProtocol')return 'A runtime protocol message could not be handled.';
  if(entry.kind==='transport')return 'The runtime transport failed.';
  if(entry.kind==='spawn')return 'The selected runtime process could not start.';
  if(entry.kind==='timeout')return 'Runtime initialization timed out.';
  if(entry.kind==='unsupportedVersion')return 'The executable did not report a supported runtime version.';
  return 'An application operation could not be completed.';
}
export function createDiagnostics() {
  let snapshot:Snapshot={entries:[],omitted:0};let serial=0;let projectSerial=0;
  const listeners=new Set<()=>void>();
  const contexts=new Map<string,Context>();
  let recent:Event[]=[];
  function emit(){for(const listener of listeners)listener();}
  function context(args:Record<string,unknown>):Context {
    const root=typeof args.root==='string'&&args.root.length<=4096?args.root:'';
    let rt=runtime(args.runtime);
    if(rt==='unknown'&&root){const matches=[...contexts].filter(([key])=>JSON.parse(key)[1]===root);if(matches.length===1)rt=matches[0][1].runtime;}
    const key=JSON.stringify([rt,root]);
    let previous=root?contexts.get(key):undefined;
    if(!previous&&root){previous={runtime:rt,project:`project-${++projectSerial}`,generation:null,state:'unknown',activeTurns:0,pendingRequests:0};contexts.set(key,previous);if(contexts.size>32)contexts.delete(contexts.keys().next().value!);}
    return {...(previous??{runtime:rt,project:'not reported',generation:null,state:'unknown',activeTurns:0,pendingRequests:0}),...(generation(args.generation)!==null?{generation:generation(args.generation)}:{})};
  }
  function remember(event:Event){recent=[...recent,event].slice(-32);}
  function failure(command:string,args:Record<string,unknown>|undefined,error:unknown){
    const a=args??{},e=obj(error),action=obj(a.action);
    const op=command==='connection state'?'connection state':commands.has(command)?command:'other';
    const operation=op==='conversation_request'&&typeof action.type==='string'&&actions.has(action.type)?`${op}/${action.type}`:op;
    const kind=typeof e.kind==='string'&&knownKinds.has(e.kind)?e.kind:'unknown';
    const detail=obj(e.detail);const rawCode=e.code??detail.code;
    const code=typeof rawCode==='number'&&Number.isSafeInteger(rawCode)&&Math.abs(rawCode)<=2147483647?rawCode:null;
    const c=context(a),at=new Date().toISOString();
    const significant=significantKinds.has(kind)||(op==='project_view'&&kind!=='cancelled');
    const event={...c,at,operation,kind};
    const index=snapshot.entries.findIndex(d=>d.operation===operation&&d.kind===kind&&d.code===code&&d.runtime===c.runtime&&d.project===c.project&&d.generation===c.generation);
    if(index>=0){snapshot={...snapshot,entries:snapshot.entries.map((d,i)=>i===index?{...d,lastSeen:at,count:Math.min(d.count+1,999999)}:d)};}
    else {
      const entry:Diagnostic={...event,id:++serial,firstSeen:at,lastSeen:at,count:1,significant,dismissed:false,code,related:recent.filter(r=>r.project===c.project&&r.runtime===c.runtime&&r.generation===c.generation).slice(-5)};
      const all=[...snapshot.entries,entry];snapshot={entries:all.slice(-32),omitted:snapshot.omitted+(all.length>32?1:0)};
    }
    remember(event);emit();
  }
  function observe(value:unknown){
    const projects=obj(value).projects;if(!Array.isArray(projects))return;
    for(const raw of projects.slice(0,32)){
      const p=obj(raw);if(typeof p.root!=='string'||p.root.length>4096)continue;
      const rt=runtime(p.runtime??'codex');
      const before=context({root:p.root,runtime:rt});
      const next={...before,generation:generation(p.generation),state:state(p.status),activeTurns:count(p.activeTurns),pendingRequests:count(p.pendingRequests)};
      contexts.set(JSON.stringify([rt,p.root]),next);
      if(before.generation!==next.generation||before.state!==next.state){
        remember({...next,at:new Date().toISOString(),operation:'connection state',kind:next.state});
        if(next.state==='failed')failure('connection state',{root:p.root,runtime:rt,generation:next.generation},{kind:typeof obj(p.error).kind==='string'&&significantKinds.has(String(obj(p.error).kind))?obj(p.error).kind:'serverExit'});
      }
    }
  }
  return {failure,observe,getSnapshot:()=>snapshot,subscribe:(listener:()=>void)=>{listeners.add(listener);return()=>{listeners.delete(listener);};},dismiss:(id:number)=>{snapshot={...snapshot,entries:snapshot.entries.map(d=>d.id===id?{...d,dismissed:true}:d)};emit();}};
}
export const diagnostics=createDiagnostics();
export function diagnosticsPrompt(entry:Diagnostic):string {
  // Build a fresh projection; never serialize unknown properties supplied by a caller.
  const facts={firstSeen:entry.firstSeen,lastSeen:entry.lastSeen,occurrences:entry.count,operation:entry.operation,category:entry.kind,message:diagnosticSummary(entry),rpcCode:entry.code,runtime:entry.runtime,project:entry.project,generation:entry.generation,connectionState:entry.state,reportedActiveTurns:entry.activeTurns,pendingRequests:entry.pendingRequests,relatedEvents:entry.related.map(e=>({at:e.at,operation:e.operation,category:e.kind,connectionState:e.state}))};
  return 'Investigate this CorpsTex application failure. Separate observed facts from hypotheses; explain likely causes and propose manual checks. Ask before changing files, permissions, credentials, or restarting processes. Do not infer that interrupted work continues.\n\nObserved client metadata (times are client receipt times):\n'+JSON.stringify(facts,null,2)+'\n\nCause: not established. Paths and IDs use temporary aliases. Messages, prompts, commands, files, output, credentials and raw payloads were omitted. Evidence is bounded and only available in this app window until reload/exit. This prompt has not been submitted to any agent.';
}
