import { ConversationLibrary } from './ConversationLibrary';
import { useCallback } from 'react';
import { ActivityBar, activityState } from './ActivityBar';
import { ComposerInput, referencedSkills, type Skill } from './ComposerInput';
import './composer-tools.css';
import { UsageRail, type TokenUsage } from './UsageRail';
import { ModelPicker, recommendedFreeModels } from './ModelPicker';
import { Settings, RuntimeSettings } from './Settings';
import { parseModelChoice, type Preferences } from './preferences';
import { InlineApprovals, matchingApprovals, type Approval } from './Approvals';
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { RichContent } from './RichContent';
import { ImageLoader, MessageImage, type ImageAttachment } from './Media';
import { ExecutionTimeline, type TimelineMode, type TimelineProject } from './Timeline';
import { ThreadControls, type ThreadOperation } from './ThreadControls';
import { RuntimeItemDetails, TurnActivity, type TurnActivityData } from './RuntimeDetails';
import { McpStatus } from './McpStatus';

type Project = { runtime?: 'codex'|'opencode'; root: string; generation: number; status: string; pendingRequests: number; activeTurns?:number; uncertain?:boolean };
type MediaContent = {type: string; text?: string; url?: string; path?: string; data?: string; mimeType?: string; imageUrl?: string; image_url?: string; resource?: {mimeType?:string;blob?:string}};
type Item = { id: string; type: string; text?: string; phase?: string; summary?: string[]; summaryDelta?: Record<string, string>; contentDelta?: Record<string, string>; content?: (MediaContent | string)[]; result?: {content?: MediaContent[]} | string; savedPath?: string; failure?: {type?:string}; fileId?: string; path?: string; contentItems?: MediaContent[]; output?: MediaContent[] | string; reported?:unknown; reportedContent?:unknown; command?: string; status?: string };
type Turn = TurnActivityData & { id: string; status: string; items: Item[]; error?: {message?: string} | null };
export type Thread = { id: string; name?: string | null; preview?: string; updatedAt?:number|string; createdAt?:number|string; turns?: Turn[]; tokenUsage?: TokenUsage; clientRevision?: number; clientArchived?:boolean; parentThreadId?:string|null; forkedFromId?:string|null; agentNickname?:string|null; agentRole?:string|null; clientRuntimeNotice?:{kind:string;message:string;willRetry:boolean} };
export type Model = { provider?:string; freeLabel?:boolean; model: string; displayName: string; isDefault?: boolean; supportedReasoningEfforts: {reasoningEffort: string}[] };
export function validReasoningEffort(model: Model | undefined, effort: string): string {
  return model?.supportedReasoningEfforts.some(option => option.reasoningEffort === effort) ? effort : '';
}

type Page<T> = { data: T[]; nextCursor?: string | null };
type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export function itemText(item: Item): string {
  if (item.type === 'userMessage') return (item.content ?? []).map(content => typeof content === 'string' ? content : content.type === 'text' ? content.text ?? '' : ['image','localImage'].includes(content.type) ? '' : `[${content.type}]`).join('\n');
  if (item.type === 'agentMessage') return item.text ?? '';
  if (item.type === 'plan') return item.text ?? '';
  if (item.type === 'enteredReviewMode' || item.type === 'exitedReviewMode') return 'review' in item && typeof item.review==='string' ? item.review : item.type;
  return `${item.type}${item.command ? `: ${item.command}` : ''}${item.status ? ` · ${item.status}` : ''}`;
}
function reasoningBlocks(base: string[], deltas: Record<string, string> = {}): string[] {
  const blocks: Record<string, string> = Object.fromEntries(base.map((text, index) => [String(index), text]));
  for (const [index, text] of Object.entries(deltas)) blocks[index] = (blocks[index] ?? '') + text;
  return Object.entries(blocks).sort(([a], [b]) => Number(a) - Number(b)).map(([, text]) => text).filter(Boolean);
}

export function ChatMessage({ item, streaming = false, runtime = 'codex' }: { item: Item; streaming?: boolean; runtime?:string }) {
  if (item.type === 'opencodeTool' || item.type === 'opencodeUpdate') return <article className="message message-activity"><div className="message-heading"><span className="message-role">OpenCode activity</span><span>{item.status}</span></div><pre className="message-body">{JSON.stringify(item.reported,null,2)}</pre></article>;
  if (item.type === 'reasoning') {
    const summary = reasoningBlocks(item.summary ?? [], item.summaryDelta);
    const content = reasoningBlocks((item.content ?? []).filter((entry): entry is string => typeof entry === 'string'), item.contentDelta);
    const text = [...summary, ...content].join('\n\n');
    return <article className="message message-reasoning"><details open>
      <summary><span className="message-role">Reasoning</span><span className="message-kind">Reported by {runtime==='opencode'?'OpenCode':'Codex'}</span></summary>
      <div className="message-body">{text ? <pre>{text}</pre> : <p className="reasoning-empty">No reasoning text reported yet.</p>}</div>
    </details></article>;
  }
  const media: {source:string;fallback?:string;alt:string}[] = [];
  if (item.type === 'userMessage') for (const content of item.content ?? []) {
    if (typeof content !== 'string' && (content.type === 'image' || content.type === 'localImage')) media.push({source:content.url ?? content.path ?? '',alt:'Attached image'});
  }
  if (item.type === 'imageGeneration') {
    const result = typeof item.result === 'string' ? item.result : '';
    if (item.savedPath || result) media.push({source:result ? (result.startsWith('data:') ? result : `data:image/png;base64,${result}`) : item.savedPath!,fallback:result ? item.savedPath : undefined,alt:'Generated image'});
  }
  if (item.type === 'imageView' && item.path) media.push({source:item.path,alt:'Viewed image'});
  if (item.type === 'mcpToolCall' && typeof item.result === 'object') for (const content of item.result?.content ?? []) {
    if (content.type === 'image' && content.data && content.mimeType) media.push({source:`data:${content.mimeType};base64,${content.data}`,alt:'Tool image'});
    if(content.type==='resource' && content.resource?.blob && content.resource.mimeType) media.push({source:`data:${content.resource.mimeType};base64,${content.resource.blob}`,alt:'Tool image'});
  }
  if (item.type === 'dynamicToolCall' || item.type === 'functionCallOutput') for (const content of item.type === 'dynamicToolCall' ? item.contentItems ?? [] : Array.isArray(item.output) ? item.output : []) {
    const source=content.type==='inputImage'?content.imageUrl:content.type==='input_image'?content.image_url:undefined;
    if(source) media.push({source,alt:'Tool image'});
  }
  const user = item.type === 'userMessage';
  const assistant = item.type === 'agentMessage';
  return <article data-streaming={streaming || undefined} className={`message message-${user ? 'userMessage' : assistant ? 'agentMessage' : 'activity'}`}>
    <div className="message-heading"><span className="message-role">{user ? 'You' : assistant ? runtime==='opencode'?'OpenCode':'Codex' : 'Activity'}</span>{assistant && item.phase === 'commentary' && <span className="message-kind">Progress</span>}</div>
    {assistant||item.type==='plan' ? <RichContent text={itemText(item)} streaming={streaming}/> : <pre className="message-body">{itemText(item)}</pre>}
    {!user&&!assistant&&item.type!=='plan'&&<RuntimeItemDetails item={item}/>}
    {item.type==='imageGeneration' && !media.length && <p role="status">{item.status==='inProgress'?'Generating image…':item.failure?.type?`Codex reported an image generation failure: ${item.failure.type}.`:item.fileId?'Codex returned an image reference without preview data or a local file.':'Codex did not provide image preview data or a local file.'}</p>}
    {item.reportedContent != null && <pre className="message-body">{JSON.stringify(item.reportedContent,null,2)}</pre>}
    {media.map((image,index)=><MessageImage key={index} source={image.source} fallback={image.fallback} alt={image.alt}/>)}
  </article>;
}

export function shouldSendMessage(event: {key: string; shiftKey: boolean; altKey: boolean; ctrlKey: boolean; metaKey: boolean; isComposing: boolean}): boolean {
  return event.key === 'Enter' && event.shiftKey && !event.isComposing && !event.altKey && !event.ctrlKey && !event.metaKey;
}

// Draft edits and project polling must not rebuild a long transcript.
export const ThreadTranscript = memo(function ThreadTranscript({ turns, runtime = 'codex', completedTurn = null }: { turns: Turn[]; runtime?:string; completedTurn?:string|null }) {
  return <>{turns.map(turn => {
    const streamingId = turn.status === 'inProgress' ? [...(turn.items ?? [])].reverse().find(item => item.type === 'agentMessage')?.id : undefined;
    return <section className="turn" data-completed={turn.id===completedTurn || undefined} key={turn.id} aria-label={`Turn ${turn.status}`}>
    <TurnActivity turn={turn}/>
    {(turn.items ?? []).map(item => <ChatMessage key={item.id} item={item} streaming={item.id === streamingId} runtime={runtime}/>)}
    <p className="turn-state" role="status">{turn.status}{turn.error?.message ? ` · ${turn.error.message}` : ''}{turn.id===completedTurn&&<span className="turn-complete-cue" aria-hidden="true">✓</span>}</p>
  </section>;
  })}</>;
});

export function Conversation({ sidebarToggle, runtime = 'codex', runtimeControls, onDraftState, project, controls, connect, onChooseProject, invoke, errorText, settingsOpen, onCloseSettings, preferences, onPreferences, storageError, approvals, approvalBusy, approvalInvoke, exiting, timelineProjects = [], sessionId = '', initialThread, autoOpen = 0, onOpenSession, onSessionState, timelineMode = 'collapsed', onTimelineModeChange, showTimeline = true, visible = true, editorRoot = '' }: { sidebarToggle?:ReactNode; editorRoot?:string; visible?:boolean; sessionId?:string; initialThread?:string; autoOpen?:number; onOpenSession?:(thread?:Thread)=>void; onSessionState?:(thread:Thread|null,pending:boolean)=>void; timelineMode?:TimelineMode; onTimelineModeChange?:(mode:TimelineMode)=>void; showTimeline?:boolean; runtime?: 'codex'|'opencode'; runtimeControls?:ReactNode; onDraftState?:(pending:boolean)=>void; project: Project | null; controls: ReactNode; connect: () => Promise<boolean>; onChooseProject: () => void; invoke: Invoke; errorText: (error: unknown) => string; settingsOpen:boolean; onCloseSettings:()=>void; preferences:Preferences; onPreferences:(next:Preferences)=>void; storageError:string; approvals:Approval[]; approvalBusy:Set<string>; approvalInvoke:Invoke; exiting:boolean; timelineProjects?:TimelineProject[] }) {
  const ids = (name:string) => sessionId ? `${sessionId}-${name}` : name;
  const container = useRef<HTMLDivElement>(null);
  const runtimeName=runtime==='opencode'?'OpenCode':'Codex';
  const [configOptions,setConfigOptions]=useState<{id:string;currentValue:string;options:{value:string;name:string}[]}[]>([]);
  const [listing,setListing] = useState(false);
  const listRequest = useRef<symbol|null>(null);
  const [threads, setThreads] = useState<Thread[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [archivedList,setArchivedList]=useState(false);
  const [allSources,setAllSources]=useState(false);
  const historyMode=archivedList?'archived':allSources?'all':'active';
  const historyView=useRef(historyMode);historyView.current=historyMode;
  const [selected, setSelected] = useState<string | null>(null);
  const [thread, setThread] = useState<Thread | null>(null);
  const [historyCursor, setHistoryCursor] = useState<string | null>(null);
  const [modelsLoading,setModelsLoading]=useState(false);
  const [models, setModels] = useState<Model[]>([]);
  const [model, setModel] = useState('');
  const [effort, setEffort] = useState('');
  const [catalogError, setCatalogError] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState('');
  const [readingImages,setReadingImages] = useState(false);
  const [images,setImages] = useState<ImageAttachment[]>([]);
  const imagesRef=useRef(images);imagesRef.current=images;
  const [attachedSkills, setAttachedSkills] = useState<Skill[]>([]);
  const [draftTurn, setDraftTurn] = useState<string | null | undefined>(undefined);
  const [delivery, setDelivery] = useState('');

  const submitting = useRef(false);
  const draftRef = useRef(draft); draftRef.current = draft;
  const [wantNew, setWantNew] = useState(false);
  const binding = `${runtime}:${project?.root ?? ''}:${project?.generation ?? 0}`;
  const threadBinding = useRef('');
  const current = useRef(binding); current.current = binding;
  const opened = useRef<Thread[]>([]);
  const messageEnd = useRef<HTMLDivElement>(null);
  const transcript = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const interrupting = useRef(false);
  const [follow, setFollow] = useState(true);
  function setFollowing(value:boolean) { following.current=value;setFollow(value); }
  const liveTurn = useRef<{scope:string;id:string}|null>(null);
  const [completedTurn,setCompletedTurn] = useState<string|null>(null);
  const ready = project?.status === 'ready';
  const turns = thread?.turns ?? [];
  const active = [...turns].reverse().find(turn => turn.status === 'inProgress');
  const targetTurn = runtime==='opencode' ? null : draftTurn === undefined ? active?.id ?? null : draftTurn;
  const staleTarget = Boolean(targetTurn && active?.id !== targetTurn);
  const catalog = models.find(entry => entry.model === model);
  const defaultModel = models.find(entry => entry.isDefault);
  const reasoningModel = catalog ?? defaultModel;

  useEffect(() => {
    if (models.length) setEffort(previous => validReasoningEffort(reasoningModel, previous));
  }, [reasoningModel, models.length]);

  async function request<T>(action: Record<string, unknown>): Promise<T> {
    if (!project || !ready) throw {kind: 'serverExit'};
    return invoke<T>('conversation_request', {runtime, root: project.root, generation: project.generation, action});
  }
  async function list(more = false, reset = false) {
    if(listRequest.current)return;
    const key = binding;
    const mode=historyMode;
    const requestId=Symbol();listRequest.current=requestId;setListing(true);
    try {
      const page = await request<Page<Thread>>({type:archivedList?'listArchivedThreads':allSources?'listAllThreads':'listThreads',cursor:more ? cursor : null});
      if (current.current !== key || historyView.current!==mode || listRequest.current!==requestId) return;
      setThreads(previous => {
        const base=[...(more||!reset?previous:[]),...opened.current.filter(local=>Boolean(local.clientArchived)===archivedList)];
        const byId=new Map(base.map(thread=>[thread.id,thread]));
        for(const thread of page.data)byId.set(thread.id,thread);
        return [...byId.values()];
      });
      if(more||reset||threads.length===0)setCursor(page.nextCursor && page.nextCursor!==(more?cursor:null) ? page.nextCursor : null);
    } catch (error) { if (current.current === key && historyView.current===mode && listRequest.current===requestId) setError(errorText(error)); }
    finally {if(listRequest.current===requestId){listRequest.current=null;setListing(false);}}
  }
  async function loadModels(sessionId:string|null=selected) {
    const key = binding;setModelsLoading(true);
    try {
      let cursor: string | null = null;
      const data: Model[] = [];
      do {
        const page: Page<Model> & {configOptions?:typeof configOptions} = await request({type:'listModels',cursor,threadId:sessionId});
        if (current.current !== key) return;
        if(page.configOptions)setConfigOptions(page.configOptions);
        data.push(...page.data); cursor = page.nextCursor ?? null;
      } while (cursor);
      setModels(data); setCatalogError('');
      setModel(previous => data.some(entry => entry.model === previous) ? previous : runtime==='opencode' ? recommendedFreeModels(data)[0]?.model??'' : '');
    } catch (error) { if (current.current === key) {setModels([]);setModel('');setEffort('');setCatalogError(errorText(error));} }
    finally {if(current.current===key)setModelsLoading(false);}
  }
  useEffect(() => {
    threadBinding.current = binding;
    opened.current = [];
    setThreads([]); setSelected(null); setThread(null); setCursor(null); setHistoryCursor(null); setModels([]);setModelsLoading(false); let choice = {model:'',effort:''}; try { choice=parseModelChoice(localStorage.getItem(runtime==='codex'?`corpstex.model:${project?.root}`:`corpstex.model:${runtime}:${project?.root}`)); } catch {} setModel(choice.model); setEffort(choice.effort); setError(''); setBusy(false); setAttachedSkills([]); setImages([]); setDraft(''); setDraftTurn(undefined); setDelivery('');
    // A server generation is the identity of a connection; old callbacks cannot reroute new work.
  }, [binding]);
  useEffect(() => {listRequest.current=null;setListing(false);if (ready && visible) {void list();void loadModels();}}, [binding, ready, visible, historyMode]);
  useEffect(() => {
    if (!selected || !project || !visible) return;
    let disposed = false; let inFlight = false; let after: number | null = null;
    async function refresh() {
      if (inFlight) return; inFlight = true;
      try {
        const view = await invoke<Thread | null>('conversation_view',{runtime,root:project!.root,generation:project!.generation,threadId:selected,after});
        if (!disposed && view) { after = view.clientRevision ?? null; setThread(view);opened.current=opened.current.map(entry=>entry.id===view.id?view:entry); }
      } catch (error) {if (!disposed) setError(errorText(error));}
      finally {inFlight = false;}
    }
    void refresh(); const timer = window.setInterval(() => {void refresh();},200);
    return () => {disposed = true;window.clearInterval(timer);};
  }, [binding, selected, invoke, errorText, visible]);
  const approvalPending=useRef(false);
  approvalPending.current=matchingApprovals(approvals,project,selected).length>0;
  useLayoutEffect(() => {
    const node = messageEnd.current?.parentElement;
    if (!visible || !follow || !node || approvalPending.current) return;
    let frame:number|null=null;
    function schedule() {
      if(frame!==null)return;
      frame=requestAnimationFrame(()=>{
        frame=null;
        if(!following.current || approvalPending.current)return;
        if(node!.scrollHeight-node!.scrollTop-node!.clientHeight>=2) {
          node!.scrollTo({top:node!.scrollHeight,behavior:'instant'});
        }
      });
    }
    // Track actual layout too (images, expanded details, composer/window resize).
    // Snapshot updates must not restart browser smooth-scroll animations.
    const observer=new ResizeObserver(schedule);
    observer.observe(node);
    if(transcript.current)observer.observe(transcript.current);
    schedule();
    return ()=>{observer.disconnect();if(frame!==null)cancelAnimationFrame(frame);};
  }, [thread, follow, visible]);
  const lastTurn = turns[turns.length - 1];
  useEffect(()=>{
    const scope=JSON.stringify([binding,selected]);
    const previous=liveTurn.current;
    liveTurn.current=visible&&ready&&lastTurn?.status==='inProgress'?{scope,id:lastTurn.id}:null;
    if(!visible||!ready||previous?.scope!==scope){setCompletedTurn(null);return;}
    if(previous && previous.id===lastTurn?.id && lastTurn.status==='completed' && !lastTurn.error) {
      const reduced=preferences.motion==='reduced'||window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      setCompletedTurn(reduced?null:lastTurn.id);
    }
  },[binding,selected,visible,ready,lastTurn?.id,lastTurn?.status,lastTurn?.error,preferences.motion]);
  useEffect(()=>{
    if(!completedTurn)return;
    // End on the actual animation, rather than expiring before a busy WebView
    // has painted it. Hidden documents and motion changes consume the cue.
    const media=window.matchMedia('(prefers-reduced-motion: reduce)');
    const clear=()=>setCompletedTurn(null);
    const hidden=()=>{if(document.hidden)clear();};
    media.addEventListener('change',clear);
    document.addEventListener('visibilitychange',hidden);
    hidden();
    return ()=>{media.removeEventListener('change',clear);document.removeEventListener('visibilitychange',hidden);};
  },[completedTurn]);
  useEffect(() => {
    if (wantNew && project?.status === 'failed') {setWantNew(false);setError(`${runtimeName} could not start. Open Project folder to review the connection error and reconnect.`);}
  }, [wantNew, project?.status]);
  useEffect(() => {if (ready && wantNew && !busy) {setWantNew(false);void open();}}, [ready, wantNew, busy]);

  useEffect(()=>onDraftState?.(Boolean(draft||images.length||readingImages)),[draft,images.length,readingImages,onDraftState]);
  const sessionPending=Boolean(draft||images.length||readingImages||busy);
  useEffect(()=>onSessionState?.(thread,sessionPending),[thread,sessionPending]);
  const attempted = useRef(0);
  useEffect(()=>{if(ready && autoOpen && attempted.current!==autoOpen){attempted.current=autoOpen;if(initialThread && selected===initialThread)return;void open(initialThread?{id:initialThread}:undefined,true);}},[ready,binding,autoOpen]);
  useEffect(() => {
    if (ready && lastTurn && lastTurn.status !== 'inProgress') {void list();}
  }, [ready, lastTurn?.id, lastTurn?.status]);

  async function open(existing?: Thread, direct = false) {
    if(exiting)return;
    if(!direct && onOpenSession && (existing || selected || initialThread) && existing?.id!==selected){onOpenSession(existing);return;}
    if (existing?.id === selected) { document.getElementById(ids('message'))?.focus(); return; }
    const key = binding;
    if (!ready) {setWantNew(true);try {if (!await connect()) setWantNew(false);}catch (error){setWantNew(false);setError(errorText(error));}return;}
    setBusy(true);setError('');
    try {
      if(existing && runtime==='codex' && existing.clientArchived===undefined) {
        const view=await invoke<Thread|null>('conversation_view',{runtime,root:project!.root,generation:project!.generation,threadId:existing.id,after:null});
        if(current.current!==key)return;
        if(view?.clientArchived)existing=view;
      }
      const result = existing?.clientArchived ? {thread:existing} : await request<{thread: Thread}>({type:existing ? 'resume' : 'newThread',...(existing ? {threadId:existing.id} : {})});
      if (current.current !== key) return;
      opened.current = [result.thread,...opened.current.filter(entry => entry.id !== result.thread.id)];
      if(runtime==='opencode')void loadModels(result.thread.id);
      setSelected(result.thread.id);setThread(result.thread);setAttachedSkills([]);setImages([]);setDraft('');setDraftTurn(undefined);setDelivery('');setHistoryCursor(null);setFollowing(true);
      setThreads(previous => [result.thread,...previous.filter(entry => entry.id !== result.thread.id)]);
      if (existing) {
        const history = await request<Page<Turn>>({type:'history',threadId:result.thread.id,cursor:null});
        if (current.current === key) setHistoryCursor(history.nextCursor ?? null);
      }
    } catch (error) {if(current.current===key)setError(errorText(error));}
    finally {if(current.current===key)setBusy(false);}
  }
  async function send() {
    if (/^\s*\//.test(draft)) { setError('Choose a command from the picker. Unsupported slash commands are not sent as chat.'); return; }
    if (thread?.clientArchived || exiting || !selected || !ready || busy || readingImages || submitting.current || (!draft.trim() && !images.length) || staleTarget || (!targetTurn && active)) return;
    const text = draft; const sentImages = images; const key = binding; const id = selected; const turnId = targetTurn;
    submitting.current = true; setBusy(true);setError('');setDelivery('');
    try {
      if (turnId) {
        await request({type:'steer',threadId:id,expectedTurnId:turnId,text,images:sentImages.map(image=>image.url),skills:referencedSkills(text,attachedSkills)});
      } else {
        await request({type:'send',threadId:id,text,images:sentImages.map(image=>image.url),skills:referencedSkills(text,attachedSkills),model:model || (effort ? reasoningModel?.model : null) || null,effort:effort || null});
      }
      if(current.current===key) {

        if (draftRef.current === text && imagesRef.current === sentImages) { setAttachedSkills([]); setImages([]); setDraft(''); setDraftTurn(undefined); }
        setDelivery(turnId ? 'Steering accepted for this turn.' : `Message accepted by ${runtimeName}.`);
        void list();
      }
    } catch(error){if(current.current===key)setError(`${errorText(error)} Your draft is kept. Check the destination and reported turn state before retrying.`);}
    finally{submitting.current=false;if(current.current===key)setBusy(false);}
  }
  async function interrupt() {
    if(!selected || !active || !ready || exiting || interrupting.current)return;
    const key=binding;const threadId=selected;
    interrupting.current=true;
    try{await request({type:'interrupt',threadId,turnId:active.id});}
    catch(error){if(current.current===key)setError(errorText(error));}
    finally{interrupting.current=false;}
  }
  useEffect(()=>{
    if(!visible||settingsOpen||exiting||!ready||busy||!active)return;
    function stopShortcut(event:KeyboardEvent) {
      if(!['Escape','Esc'].includes(event.key)||event.defaultPrevented||event.repeat||event.isComposing||event.altKey||event.ctrlKey||event.metaKey||event.shiftKey)return;
      // Capture before textarea/WebView handlers consume Escape. Dismissible
      // surfaces retain it, including trusted controls behind closed shadow DOM.
      if(document.querySelector('dialog[open],[data-menu-open],[aria-modal="true"],.project-picker:not([hidden])')||container.current?.querySelector('.composer-picker'))return;
      if(event.composedPath().some(node=>node instanceof Element&&node.matches('.inline-approvals,[data-approval-controls],.thread-search,[contenteditable="true"]')))return;
      event.preventDefault();
      event.stopPropagation();
      void interrupt();
    }
    window.addEventListener('keydown',stopShortcut,true);
    return ()=>window.removeEventListener('keydown',stopShortcut,true);
  },[visible,settingsOpen,exiting,ready,busy,active?.id,binding,selected]);
  async function older() {
    if (!selected || !historyCursor)return;
    try{const page=await request<Page<Turn>>({type:'history',threadId:selected,cursor:historyCursor});setHistoryCursor(page.nextCursor ?? null);}
    catch(error){setError(errorText(error));}
  }
  async function threadAction(action:ThreadOperation):Promise<boolean> {
    if(runtime!=='codex'||!selected||!ready||busy||active||exiting)return false;
    const key=binding;const threadId=selected;
    setBusy(true);setError('');setDelivery('');
    try {
      const result=await request<{thread?:Thread}>({...action,threadId});
      if(current.current!==key)return false;
      if(action.type==='fork'&&result.thread) {
        if(onOpenSession)onOpenSession(result.thread);
        setDelivery(`Fork created: ${result.thread.id}. The source conversation is unchanged.`);
      } else if(action.type==='archive'||action.type==='unarchive') {
        const archived=action.type==='archive';
        opened.current=opened.current.map(entry=>entry.id===threadId?{...entry,clientArchived:archived}:entry);
        setThread(previous=>previous?.id===threadId?{...previous,clientArchived:archived}:previous);
        setThreads(previous=>previous.filter(entry=>entry.id!==threadId));
        if(!archived)await request({type:'resume',threadId});
        if(current.current!==key)return false;
        setDelivery(archived?'Archived in Codex. Restore this conversation through Thread actions or Archived.':'Conversation restored.');
      } else if(action.type==='rename') {
        const rename=(entry:Thread)=>entry.id===threadId?{...entry,name:action.name}:entry;
        opened.current=opened.current.map(rename);setThreads(previous=>previous.map(rename));setThread(previous=>previous?rename(previous):previous);
        setDelivery('Conversation renamed.');
      } else setDelivery(action.type==='compact'?'Compaction requested. Follow runtime progress in the conversation and timeline.':'Review accepted. Follow the review turn in this conversation.');
      return true;
    } catch(error) {if(current.current===key)setError(`${errorText(error)} The thread action was not confirmed. Refresh before retrying; your draft is kept.`);return false;}
    finally{if(current.current===key)setBusy(false);}
  }
  function rememberChoice(nextModel: string, nextEffort: string) {
    if (!project) return;
    try { localStorage.setItem(runtime==='codex'?`corpstex.model:${project.root}`:`corpstex.model:${runtime}:${project.root}`,JSON.stringify({model:nextModel,effort:nextEffort})); }
    catch {setError('Model choices apply now, but could not be saved on this device.');}
  }
  async function chooseModel(value: string) {
    const key=binding;
    if(runtime==='opencode' && selected && value) {
      setBusy(true);setCatalogError('');
      try {await request({type:'openCodeConfig',threadId:selected,configId:'model',value});if(current.current!==key)return;setModel(value);setEffort('');rememberChoice(value,'');await loadModels();}
      catch(error){if(current.current===key)setCatalogError(errorText(error));}
      finally{if(current.current===key)setBusy(false);}
    } else {setModel(value);setEffort('');rememberChoice(value,'');}
  }
  async function chooseMode(value:string) {
    const key=binding;if(!selected)return;setBusy(true);
    try {await request({type:'openCodeConfig',threadId:selected,configId:'mode',value});if(current.current===key)await loadModels();}
    catch(error){if(current.current===key)setError(errorText(error));}
    finally{if(current.current===key)setBusy(false);}
  }
  function chooseEffort(value: string) {setEffort(value);rememberChoice(model,value);}
  const modelControls = (<div className="model-controls" role="group" aria-label="Settings for the next message">
            {runtimeControls}
            <ModelPicker loading={modelsLoading} models={models} value={model} disabled={!ready || exiting || busy || modelsLoading || !models.length || (runtime==='opencode' && Boolean(active))} runtimeName={runtimeName} onChange={value=>void chooseModel(value)}/>

            <label>Reasoning<select aria-label="Reasoning effort" value={effort} disabled={!ready || !reasoningModel?.supportedReasoningEfforts.length || busy || (runtime==='opencode' && Boolean(active))} onChange={event=>chooseEffort(event.target.value)}><option value="">{runtimeName} default</option>{reasoningModel?.supportedReasoningEfforts.map(option=><option key={option.reasoningEffort} value={option.reasoningEffort}>{option.reasoningEffort}</option>)}</select></label>
            <span className="settings-hint">{active ? 'Applies to the next turn.' : !ready ? 'Connect a project first.' : !models.length ? 'No models available. Refresh to retry.' : !reasoningModel ? 'Choose a model first.' : !reasoningModel.supportedReasoningEfforts.length ? 'This model has no reasoning options.' : ''}</span>
            {runtime==='opencode' && selected && configOptions.find(c=>c.id==='mode') && <label>Mode<select aria-label="OpenCode session mode" disabled={!ready||busy||Boolean(active)} value={configOptions.find(c=>c.id==='mode')?.currentValue??''} onChange={event=>void chooseMode(event.target.value)}>{configOptions.find(c=>c.id==='mode')?.options.map(o=><option key={o.value} value={o.value}>{o.name}</option>)}</select></label>}
          </div>);
  const inlineApprovals = matchingApprovals(approvals, project, selected);
  const activity = activityState(Boolean(ready),Boolean(selected),turns,inlineApprovals.some(a=>a.status==='ready'),Boolean(error));
  function composerCommand(id:string) {
    if(id==='new') { void open(); return; }
    if(id==='stop') { if(active)void interrupt();else setDelivery('No active turn to stop.');return; }
    if(id==='review'||id==='compact') {
      if(runtime!=='codex')return;
      const details=container.current?.querySelector<HTMLDetailsElement>('.thread-controls');
      if(details){details.open=true;details.querySelector('select')?.focus();setDelivery(`Choose ${id==='review'?'Review code':'Compact context'} in Thread actions.`);}
      return;
    }
    const selector=id==='resume'?'.thread-search':id==='reasoning'?'select[aria-label="Reasoning effort"]':'.model-picker-trigger';
    container.current?.querySelector<HTMLElement>(selector)?.focus();
  }
  function changeImages(next:ImageAttachment[]) {setImages(next);setDelivery('');if(draftTurn===undefined)setDraftTurn(active?.id??null);}
  function changeDraft(value:string) {setDraft(value);setDelivery('');if(!value&&!images.length)setDraftTurn(undefined);else if(draftTurn===undefined)setDraftTurn(active?.id??null);}

  const libraryActions=useRef({open,list});libraryActions.current={open,list};
  const openLibraryThread=useCallback((entry:Thread)=>{void libraryActions.current.open(entry);},[]);
  const moreLibraryThreads=useCallback(()=>{void libraryActions.current.list(true);},[]);

  const loadImage=useMemo(()=>(path:string)=>invoke<string>('conversation_image',{runtime,root:project?.root,generation:project?.generation,threadId:selected,path}),[binding,selected,invoke]);
  return <div className="session-content" ref={container} onAnimationEnd={event=>{if(event.animationName==='turn-complete')setCompletedTurn(null);}}>
    {settingsOpen && <Settings timelineMode={timelineMode} onTimelineModeChange={onTimelineModeChange} editorRoot={editorRoot||project?.root||''} tools={<McpStatus runtime={runtime} project={project} threadId={selected} invoke={invoke} errorText={errorText}/>} onClose={onCloseSettings} preferences={preferences} onPreferences={onPreferences} storageError={storageError} modelControls={<>{modelControls}<button type="button" disabled={!ready} onClick={()=>void loadModels()}>Refresh models</button>{catalogError&&<p role="alert">{catalogError}</p>}</>} runtime={<RuntimeSettings runtime={runtime} project={project} invoke={invoke} errorText={errorText}/>} />}
    <aside className="projects" aria-labelledby={ids('projects-title')}>
      <div className="sidebar-header">{controls}{sidebarToggle}</div>
      <div className="thread-actions"><button type="button" disabled={busy || wantNew} onClick={() => {void open();}}>{wantNew ? 'Connecting…' : 'New conversation'}</button><button type="button" aria-label="Refresh conversations" title="Refresh conversations" disabled={!ready || busy || listing} onClick={() => {void list(false,true);void loadModels();}}><svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7"><path d="M20 7v5h-5M4 17v-5h5"/><path d="M6.1 7a7 7 0 0 1 11.5-1L20 9M4 15l2.4 3A7 7 0 0 0 18 17"/></svg></button></div>
      <div className="panel-heading"><h2 id={ids('projects-title')}>Conversations</h2><span className="count">{threads.length}</span></div>

      <ConversationLibrary key={`${runtime}:${project?.root??''}`} scope={`${runtime}:${project?.root??''}`} threads={threads} selected={selected} disabled={busy||exiting} ready={ready} hasMore={Boolean(cursor)} loading={listing} onMore={moreLibraryThreads} onOpen={openLibraryThread}/>
      {runtime==='codex'&&<details className="library-sources"><summary>History sources</summary><label className="history-filter">History<select aria-label="Conversation history view" value={historyMode} disabled={busy} onChange={event=>{setArchivedList(event.target.value==='archived');setAllSources(event.target.value==='all');setThreads([]);setCursor(null);}}><option value="active">Interactive</option><option value="all">All sources · includes subagents</option><option value="archived">Archived · all sources</option></select></label></details>}

    </aside>
    <main id={ids('workspace')} className="workspace" data-working={Boolean(active)} data-approval-pending={inlineApprovals.length>0} tabIndex={-1} aria-labelledby={ids('workspace-title')}>
      <div className="workspace-atmosphere" aria-hidden="true"><i className="matrix-red"/><i className="matrix-green"/><i className="matrix-blue"/><span className="graphite-frost"/></div>
      <div className="workspace-heading"><h1 id={ids('workspace-title')}>{thread?.name || thread?.preview?.slice(0,60) || 'Conversation'}</h1>{runtime==='codex'&&thread&&<ThreadControls key={`${binding}:${thread.id}`} threadId={thread.id} name={thread.name} archived={thread.clientArchived} disabled={!ready||busy||Boolean(active)||exiting} archiveBlocked={Boolean(project?.activeTurns||project?.pendingRequests||project?.uncertain)} onAction={threadAction}/>}<span className="eyebrow">{!ready ? 'DISCONNECTED' : active ? 'WORKING' : thread?.clientArchived?'ARCHIVED':selected ? 'READY' : 'NO SESSION'}</span></div>
      {(thread?.parentThreadId||thread?.forkedFromId)&&<p className="panel-note">{thread.parentThreadId?`Parent agent: ${thread.parentThreadId}`:`Forked from: ${thread.forkedFromId}`}{thread.agentNickname?` · ${thread.agentNickname}`:''}{thread.agentRole?` · ${thread.agentRole}`:''}</p>}
      {runtime==='opencode' ? <aside className="usage-rail" aria-label="OpenCode usage">Usage and pricing are not reported by ACP. “Free” identifies catalog labels.</aside> : <UsageRail project={project} threadId={selected} tokenUsage={thread?.tokenUsage} turnState={`${turns.at(-1)?.id}:${turns.at(-1)?.status}`} invoke={invoke}/>}
      {!thread ? <div className="welcome"><div className="welcome-content"><h2>No conversation is open.</h2><p>Pick up a recent conversation, or start a new one to begin working with {runtimeName}.</p><button className="welcome-action" type="button" disabled={busy || wantNew} onClick={()=>ready ? void open() : void connect()}>{ready ? 'New conversation' : 'Open project'}</button>{initialThread && !selected && <button type="button" disabled={!ready||busy||exiting} onClick={()=>void open({id:initialThread},true)}>Resume saved session</button>}{!ready && <button type="button" disabled={busy || wantNew} onClick={onChooseProject}>Choose another folder…</button>}</div></div> : <div className="chat-stage"><div className="messages" role="region" tabIndex={0} aria-label="Conversation messages" onWheel={event=>{if(event.deltaY<0)setFollowing(false);else if(event.deltaY>0){const node=event.currentTarget;setFollowing(node.scrollHeight-node.scrollTop-node.clientHeight<2);}}} onTouchStart={()=>setFollowing(false)} onPointerDown={()=>setFollowing(false)} onKeyDown={event=>{if(event.target===event.currentTarget&&['ArrowUp','PageUp','Home','ArrowDown','PageDown','End',' '].includes(event.key))setFollowing(event.key==='End');}} onScroll={event=>{const node=event.currentTarget;if(following.current)return;setFollowing(node.scrollHeight-node.scrollTop-node.clientHeight<2);}}>
        {historyCursor && <button type="button" onClick={() => {void older();}}>Load older turns</button>}
        {turns.length===0 && <p className="conversation-empty">No messages yet.</p>}
        <div ref={transcript}><ImageLoader.Provider value={loadImage}><ThreadTranscript turns={turns} runtime={runtime} completedTurn={completedTurn}/></ImageLoader.Provider></div>
        <InlineApprovals key={`${binding}:${selected}`} approvals={inlineApprovals} exiting={exiting} busyKeys={approvalBusy} invoke={approvalInvoke} errorText={errorText} visible={visible} onReveal={()=>setFollowing(false)} />

        <div ref={messageEnd}/>
      </div></div>}
      {preferences.activityDisplay!=='hidden' && <ActivityBar preset={preferences.activityPreset} label={activity.label} active={activity.active} compact={preferences.activityDisplay==='compact'}/>}
      <div className="composer-placeholder">
        {error && <p role="alert">{error}</p>}
        {thread?.clientRuntimeNotice&&<p role="status">Latest runtime notice · {thread.clientRuntimeNotice.kind}{thread.clientRuntimeNotice.willRetry?' · retrying':''}: {thread.clientRuntimeNotice.message}</p>}
        {project && project.status !== 'ready' && selected && <p role="alert">The server connection was lost. Work will not resume automatically. Reconnect the project and resume its saved thread.</p>}
        {Boolean(project?.pendingRequests) && <p role="alert">Response needed in the Approval center.</p>}
        <form onSubmit={event=>{event.preventDefault();void send();}}>

          <div className="composer-heading"><label htmlFor={ids('message')}>Message</label>{selected && <p className="message-destination" id={ids('message-destination')}><span>{targetTurn ? 'Steering' : 'Sending to'} · {thread?.name || selected.slice(0,8)}</span><span>{project?.root}{targetTurn ? ` · Turn ${targetTurn.slice(0,8)}` : ''}</span></p>}</div><ComposerInput idPrefix={sessionId} key={`${binding}:${selected}`} value={draft} onChange={changeDraft} onSend={()=>void send()} onCommand={composerCommand} skills={attachedSkills} onSkills={setAttachedSkills} images={images} onImages={changeImages} onReading={setReadingImages} project={project} disabled={Boolean(thread?.clientArchived)||!selected||!ready||busy||(runtime==='opencode'&&Boolean(active))} invoke={invoke} errorText={errorText} selected={Boolean(selected)} trailing={modelControls} dockActions={<div className="composer-dock-actions">{staleTarget && !active && <button type="button" onClick={()=>setDraftTurn(null)}>Use as new message</button>}{active && <button type="button" className="stop-turn" title="Stop current turn (Escape)" aria-keyshortcuts="Escape" disabled={!ready || busy} onClick={()=>{void interrupt();}}>Stop turn</button>}<button type="submit" title={targetTurn?'Steer turn (Shift+Enter)':'Send message (Shift+Enter)'} disabled={Boolean(thread?.clientArchived) || !selected || !ready || busy || readingImages || (!draft.trim() && !images.length) || staleTarget || Boolean(!targetTurn && active)}>{busy ? 'Sending…' : targetTurn ? 'Steer turn' : 'Send message'}</button></div>}/>
          {catalogError && <p>Model selection unavailable: {catalogError} {runtimeName} defaults remain in use.</p>}
          <div className="composer-actions"><p id={ids('composer-help')}>{!ready?'Disconnected':!selected?'Select a conversation.':thread?.clientArchived?'Archived · restore to send messages.':staleTarget?'The target turn ended. Your draft has not been sent.':targetTurn?'Shift+Enter to steer':active?'A turn is active. Wait for it to finish before starting another.':'Shift+Enter to send'}</p></div>
          {delivery && <p className="delivery-status" role="status">{delivery}</p>}
        </form>
      </div>
    </main>
    {showTimeline && <ExecutionTimeline active={visible} mode={timelineMode} onModeChange={onTimelineModeChange} projects={timelineProjects} history={threadBinding.current === binding && selected === thread?.id ? thread : null} historyProject={project} invoke={invoke}/>}
  </div>;
}
