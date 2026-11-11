import { Startup, useWelcome } from './Startup';
import { FolderBrowser } from './FolderBrowser';
import { Diagnostics } from './Diagnostics';
import { diagnostics, isEditorCancellation } from './diagnostics';
import { usePreferences } from './preferences';
import { ApprovalCenter, ApprovalCard } from './Approvals';
import type { Approval } from './Approvals';
export { ApprovalCard };
import { SessionWorkspace } from './SessionWorkspace';
import { RuntimePicker } from './ModelPicker';
export { ChatMessage, ThreadTranscript, shouldSendMessage, validReasoningEffort } from './Conversation';
export { RichContent, ArtifactCard, safeMessageUrl } from './RichContent';
export { parseChart } from './renderData';
export { QuestionForm, completeAnswers } from './QuestionForm';
import { useCallback, useEffect, useRef, useState } from 'react';

type Project = {
  runtime?: 'codex' | 'opencode'; root: string; generation: number; status: 'starting' | 'ready' | 'failed' | 'stopped';
  version: string | null; error: { kind: string } | null; activeTurns: number; pendingRequests: number; uncertain: boolean; sequence?: number;
};
type View = { projects: Project[]; exitRequested: boolean; exitProjects: Project[]; approvals: Approval[] };
type DesktopWindow = Window & { __TAURI__?: { core: { invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T> } } };
async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const bridge = (window as DesktopWindow).__TAURI__?.core.invoke;
  try {
    const result=bridge?await bridge<T>(command,args):await Promise.reject({kind:'desktopOnly'});
    if(command==='project_view')diagnostics.observe(result);
    return result;
  } catch(error){diagnostics.failure(command,args,error);throw error;}
}
const errors: Record<string, string> = {
  openCodeUnavailable: 'OpenCode was not found on the desktop app’s absolute PATH. Windows requires opencode.exe.',
  openCodeUnsupported: 'This operation is unavailable through the installed OpenCode ACP protocol.',
  openCodeAuthRequired: 'OpenCode requires authentication. Run opencode auth login in your terminal, then refresh the connection under Settings → Runtime & permissions or reconnect your project.',
  invalidRequest: 'This thread or selection is not valid for the current project. Refresh and try again.',
  desktopOnly: 'Open the CorpsTex desktop application to connect a project.',
  invalidProject: 'Choose an existing project folder using its absolute path.',
  missingExecutable: 'Codex was not found on the desktop app’s absolute PATH. Install the official CLI separately. Windows requires codex.exe.',
  unsupportedVersion: 'The executable did not report a supported Codex CLI version.',
  spawn: 'The installed Codex process could not start.',
  transport: 'The project server encountered a transport error.',
  malformedMessage: 'Codex returned a malformed protocol message.',
  messageTooLarge: 'Codex returned a message larger than CorpsTex’s 16 MiB transport limit. The connection was stopped; saved Codex history has not been deleted.',
  unsupportedProtocol: 'The server sent an unsupported protocol message. Its connection was stopped.',
  serverExit: 'The project server exited unexpectedly.',
  timeout: 'Codex initialization timed out.',
  cancelled: 'Project startup was cancelled.',
  remote: 'Codex rejected initialization.',
  staleGeneration: 'This server has changed. Review its current status before reconnecting.',
  busy: 'The runtime is busy with a conflicting operation. Wait for it to finish, then retry.',
  exitPending: 'Finish or cancel the exit confirmation before connecting a project.',
  confirmationRequired: 'Review the interrupted work before reconnecting or exiting.',
};
function errorText(error: unknown): string {
  const kind = typeof error === 'object' && error !== null && 'kind' in error ? String(error.kind) : '';
  if (kind === 'remote' && typeof error === 'object' && error !== null && 'detail' in error) {
    const detail = error.detail as {code?: unknown};
    if (detail?.code === -32601) return 'This operation is unavailable in the installed runtime protocol.';
    if (typeof detail?.code === 'number') return `The runtime rejected this request (code ${detail.code}).`;
  }
  return errors[kind] ?? 'The desktop bridge could not complete this action.';
}

export function validateProjectRoot(root: string): string | null {
  return root.trim() ? null : 'Enter an absolute project folder path before connecting.';
}

export function ProjectStatus({ project }: { project: Project }) {
  return <>
    <strong className="project-root">{project.root}</strong>{project.runtime==='opencode' && <span>OpenCode</span>}
    <p>{project.status === 'ready' ? 'Connected' : project.status === 'starting' ? `Starting ${project.runtime==='opencode'?'OpenCode':'Codex'}…` : project.status === 'failed' ? 'Connection lost' : 'Stopped'}{project.version ? ` · ${project.version}` : ''}</p>
    {project.error && <p>{errorText(project.error)}</p>}
    {(project.activeTurns > 0 || project.pendingRequests > 0 || project.uncertain) && <p>
      Reported turns: {project.activeTurns}. Pending server requests (including approvals): {project.pendingRequests}.
      {project.pendingRequests > 0 && ' Review outstanding requests in the Approval center.'}
      {project.uncertain && ' Some activity state is unknown.'}
      {project.status === 'failed' && ' These belong to the stopped server. Work will not resume automatically.'}
    </p>}
  </>;
}

export function ShutdownDialog({ projects, onCancel, onConfirm }: { projects: Project[]; onCancel: () => void | Promise<void>; onConfirm: () => void | Promise<void> }) {
  const dialog = useRef<HTMLDivElement>(null);
  const [exiting, setExiting] = useState(false);
  const [actionError, setActionError] = useState('');
  async function act(confirm: boolean) {
    setActionError('');
    if (confirm) setExiting(true);
    try { await (confirm ? onConfirm() : onCancel()); }
    catch (error) { setActionError(errorText(error)); setExiting(false); }
  }
  useEffect(() => {
    const previous = document.activeElement;
    dialog.current?.querySelector('button')?.focus();
    return () => { if (previous instanceof HTMLElement && previous.isConnected) previous.focus(); };
  }, []);
  function handleKeys(event: React.KeyboardEvent) {
    if (event.key === 'Escape' && !exiting) { event.preventDefault(); void act(false); }
    if (event.key === 'Tab') {
      const buttons = dialog.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)');
      if (!buttons?.length) { event.preventDefault(); return; }
      const first = buttons[0]; const last = buttons[buttons.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
    }
  }
  return <div className="exit-backdrop">
    <div className="exit-dialog" ref={dialog} role="alertdialog" aria-modal="true" aria-labelledby="exit-title" aria-describedby="exit-description" onKeyDown={handleKeys}>
      <h2 id="exit-title">Stop project servers and exit?</h2>
      <p id="exit-description">Exiting stops every project server. Active work will be interrupted. Pending requests will lose their connection without an approval decision. Work does not continue after exit and will not resume automatically.</p>
      <ul>{projects.map(project => <li key={`${project.runtime}:${project.root}`}><ProjectStatus project={project} /></li>)}</ul>
      <button type="button" disabled={exiting} onClick={() => { void act(false); }}>Keep CorpsTex open</button>
      <button type="button" disabled={exiting} onClick={() => { void act(true); }}>Stop servers and exit</button>
      {exiting && <p role="status">Stopping project servers…</p>}
      {actionError && <p role="alert">{actionError}</p>}
    </div>
  </div>;
}

export default function App() {
  useEffect(()=>{
    const failure=()=>diagnostics.failure('frontend',undefined,{kind:'frontendError'});
    const report=(event:PromiseRejectionEvent)=>{if(isEditorCancellation(event.reason)){event.preventDefault();return;}failure();};
    const reportError=(event:ErrorEvent)=>{if(event.error)failure();};
    window.addEventListener('error',reportError);window.addEventListener('unhandledrejection',report);
    return()=>{window.removeEventListener('error',reportError);window.removeEventListener('unhandledrejection',report);};
  },[]);
  const [runtime, setRuntime] = useState<'codex'|'opencode'>(()=>{try{return typeof window!=='undefined'&&localStorage.getItem('corpstex.runtime')==='opencode'?'opencode':'codex';}catch{return 'codex';}});
  const runtimeName=runtime==='opencode'?'OpenCode':'Codex';
  const [view, setView] = useState<View>({ projects: [], exitRequested: false, exitProjects: [], approvals: [] });
  const { preferences, update: onPreferences, storageError } = usePreferences();
  const [browserOpen, setBrowserOpen] = useState(false);
  const [projectPickerOpen, setProjectPickerOpen] = useState(false);
  const [projectSearch, setProjectSearch] = useState('');
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(() => { try { return typeof window === 'undefined' ? true : window.innerWidth > 759; } catch { return true; } });
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey || event.key.toLowerCase() !== 'b') return;
      const target = event.target;
      if (target instanceof HTMLElement && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) return;
      event.preventDefault();
      setSidebarOpen(open => !open);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  const pendingApprovals = useRef(new Set<string>());
  const resolvedApprovals = useRef(new Set<string>());
  const [approvalBusy, setApprovalBusy] = useState<Set<string>>(new Set());
  const approvalInvoke = useCallback(async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
    if (command !== 'approval_reply') return invoke<T>(command, args);
    const key = JSON.stringify([args?.runtime ?? 'codex',args?.root,args?.generation,(args?.reply as {ticket: number})?.ticket]);
    if (pendingApprovals.current.has(key) || resolvedApprovals.current.has(key)) throw {kind:'busy'};
    pendingApprovals.current.add(key); setApprovalBusy(new Set(pendingApprovals.current));
    try {
      const result = await invoke<T>(command,args);
      resolvedApprovals.current.add(key);
      setView(previous=>({...previous,approvals:previous.approvals.filter(a=>JSON.stringify([a.runtime ?? 'codex',a.root,a.generation,a.ticket])!==key)}));
      return result;
    } finally {pendingApprovals.current.delete(key);setApprovalBusy(new Set(pendingApprovals.current));}
  }, []);
  useEffect(()=>{if(view.exitRequested){setSettingsOpen(false);setBrowserOpen(false);}},[view.exitRequested]);
  const [root, setRoot] = useState(() => {
    try { return typeof window === 'undefined' ? '' : localStorage.getItem('corpstex.lastProject') || ''; } catch { return ''; }
  });
  const rootInput = useRef<HTMLInputElement>(null);
  const [invalidRoot, setInvalidRoot] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [failure, setFailure] = useState('');
  useEffect(()=>{if(invalidRoot||failure)setProjectPickerOpen(true);},[invalidRoot,failure]);
  useEffect(()=>{if(projectPickerOpen)requestAnimationFrame(()=>{if(invalidRoot||failure)rootInput.current?.focus();else document.querySelector<HTMLInputElement>('[aria-label="Find a project"]')?.focus();});},[projectPickerOpen,invalidRoot,failure]);
  const [pollFailure, setPollFailure] = useState('');
  const [startup, setStartup] = useState<'loading' | 'ready' | 'error'>('loading');
  const [startupAttempt, setStartupAttempt] = useState(0);
  const welcome = useWelcome();
  const startupSuppressed = view.exitRequested || view.approvals.length > 0 || view.projects.some(project => project.activeTurns > 0 || project.pendingRequests > 0 || project.uncertain);
  const showStartup = !startupSuppressed && (startup !== 'ready' || welcome.open);
  function finishWelcome() {
    welcome.finish();
    requestAnimationFrame(() => document.querySelector<HTMLButtonElement>('.settings-launcher')?.focus());
  }
  const [restart, setRestart] = useState<Project | null>(null);
  const [selectedRoot, setSelectedRoot] = useState('');
  useEffect(() => {
    if ((window as DesktopWindow).__TAURI__) {
      void invoke<string>('default_project_root').then(path => setRoot(current => current || path)).catch(() => {});
    }
  }, []);
  useEffect(() => {
    if (!(window as DesktopWindow).__TAURI__) { setStartup('error'); return; }
    let disposed = false; let inFlight = false;
    // A deadline reports a stalled bridge; it never delays a successful launch.
    const deadline = window.setTimeout(() => { if (!disposed) setStartup(previous => previous === 'ready' ? previous : 'error'); }, 10000);
    async function refresh() {
      if (inFlight) return;
      inFlight = true;
      try {
        const next = await invoke<View>('project_view');
        if (!disposed) {
          const keys = new Set(next.approvals.map(a=>JSON.stringify([a.runtime ?? 'codex',a.root,a.generation,a.ticket])));
          for (const key of resolvedApprovals.current) if (!keys.has(key)) resolvedApprovals.current.delete(key);
          window.clearTimeout(deadline);
          setStartup('ready');
          setView({...next,approvals:next.approvals.filter(a=>!resolvedApprovals.current.has(JSON.stringify([a.runtime ?? 'codex',a.root,a.generation,a.ticket])))}); setPollFailure('');
        }
      } catch {
        if (!disposed) {
          window.clearTimeout(deadline);
          setStartup(previous => previous === 'ready' ? previous : 'error');
          setPollFailure('Project status is unavailable. Keep the application open until its server state can be checked.');
        }
      } finally { inFlight = false; }
    }
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, 500);
    return () => { disposed = true; window.clearInterval(timer); window.clearTimeout(deadline); };
  }, [startupAttempt]);
  async function connect(project?: Project, chosenRoot?: string) {
    const validation = validateProjectRoot(project?.root ?? chosenRoot ?? root);
    if (validation) {
      setFailure(validation); setInvalidRoot(true); rootInput.current?.focus();
      return false;
    }
    setConnecting(true); setFailure(''); setInvalidRoot(false);
    try {
      const projectResult = await invoke<Project>('connect_project', { runtime: project?.runtime ?? runtime, root: project?.root ?? chosenRoot ?? root, restart: project?.generation ?? null, acknowledgeLoss: Boolean(project) });
      setSelectedRoot(projectResult.root);
      setRoot(projectResult.root);
      try { localStorage.setItem('corpstex.lastProject', projectResult.root); } catch { /* Selection still works when storage is unavailable. */ }
      setView(await invoke<View>('project_view'));
      setRestart(null);
      return true;
    } catch (error) {
      setFailure(errorText(error));
      if (typeof error === 'object' && error !== null && 'kind' in error && error.kind === 'invalidProject') {
        setInvalidRoot(true); rootInput.current?.focus();
      }
      return false;
    }
    finally { setConnecting(false); }
  }
  async function cancelExit() {
    await invoke('cancel_exit');
    setView(await invoke<View>('project_view'));
  }
  async function confirmExit() { await invoke('confirm_exit'); }
  const runtimeProjects=view.projects.filter(project=>(project.runtime ?? 'codex')===runtime);
  const activeProject = (selectedRoot ? runtimeProjects.find(project => project.root === selectedRoot) : runtimeProjects[0]) ?? null;
  const activeRoot = activeProject?.root || root;
  const connected = runtimeProjects.filter(project => project.status === 'ready').length;
  const starting = runtimeProjects.some(project => project.status === 'starting');
  async function chooseRuntime(next:'codex'|'opencode') {
    if(next===runtime||connecting)return;
    setRuntime(next);setSettingsOpen(false);setRestart(null);setFailure('');setProjectPickerOpen(false);
    try{localStorage.setItem('corpstex.runtime',next);}catch{}
    if(!activeRoot.trim())return;
    setConnecting(true);
    try {
      const result=await invoke<Project>('connect_project',{runtime:next,root:activeRoot,restart:null,acknowledgeLoss:false});
      setSelectedRoot(result.root);setRoot(result.root);setView(await invoke<View>('project_view'));
    } catch(error){setFailure(errorText(error));}
    finally{setConnecting(false);}
  }
  return <>
    <div className="shell" inert={view.exitRequested} data-palette={preferences.palette} data-accent={preferences.accent} data-density={preferences.density} data-text-size={preferences.textSize} data-reading-width={preferences.width} data-motion={preferences.motion} data-background={preferences.background} data-texture={preferences.texture} data-surfaces={preferences.surfaces} data-sidebar={sidebarOpen ? 'open' : 'collapsed'}>
      <div className="startup-workspace" inert={showStartup}>
      <a className="skip-link" href="#workspace" onClick={event=>{event.preventDefault();document.querySelector<HTMLElement>('.session-focused .workspace')?.focus();}}>Skip to workspace</a>
      <header className="app-header">
        <button className="sidebar-toggle" type="button" aria-expanded={sidebarOpen} aria-label={sidebarOpen ? 'Collapse sidebar' : 'Expand sidebar'} title="Toggle sidebar (Ctrl+B)" onClick={()=>setSidebarOpen(open=>!open)}><svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6"><rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M9.5 4v16"/></svg></button>
        <div className="identity">
          <span className="wordmark">CorpsTex</span>
        </div>
        <Diagnostics suppressed={view.exitRequested||view.approvals.length>0||settingsOpen||browserOpen}/>
        <button className="settings-launcher" type="button" onClick={()=>setSettingsOpen(true)} aria-haspopup="dialog" aria-expanded={settingsOpen} aria-label="Personal settings" title="Personal settings"><svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6"><path d="m9 3-1 3-3 1-2 5 2 5 3 1 1 3h6l1-3 3-1 2-5-2-5-3-1-1-3Z"/><circle cx="12" cy="12" r="3.5"/></svg></button>
        {(pollFailure||!connected)&&<p className="connection" role="status">
          {pollFailure ? `${runtimeName} status unavailable` : starting ? `${runtimeName} connecting` : `${runtimeName} disconnected`}
        </p>}
      </header>

      <SessionWorkspace onShowWelcome={()=>{setSettingsOpen(false);welcome.show();}} workspaceRoot={selectedRoot || activeProject?.root || ''} workspaceProjects={view.projects} onActivate={(next,path)=>{setRuntime(next);setSelectedRoot(path);setRoot(path);}} timelineProjects={view.projects} runtime={runtime} onChooseProject={()=>setBrowserOpen(true)} settingsOpen={settingsOpen && !view.exitRequested} onCloseSettings={()=>setSettingsOpen(false)} preferences={preferences} onPreferences={onPreferences} storageError={storageError} approvals={view.approvals} approvalBusy={approvalBusy} approvalInvoke={approvalInvoke} exiting={view.exitRequested} project={activeProject} connect={() => connect()} invoke={invoke} errorText={errorText} runtimeControls={<RuntimePicker value={runtime} disabled={connecting||view.exitRequested} counts={{codex:view.projects.filter(p=>(p.runtime??'codex')==='codex'&&p.status==='ready').length,opencode:view.projects.filter(p=>p.runtime==='opencode'&&p.status==='ready').length}} onChange={next=>void chooseRuntime(next)}/>} controls={<><button className="choose-project" type="button" aria-expanded={projectPickerOpen} aria-controls="project-picker" title={activeRoot || 'Choose project folder'} disabled={connecting||starting} onClick={()=>setProjectPickerOpen(open=>!open)}><span>Project</span><strong>{activeRoot.split(/[\\/]/).filter(Boolean).at(-1)||'Choose folder'}</strong><span aria-hidden="true">⌄</span></button>
        <div id="project-picker" className="project-picker" hidden={!projectPickerOpen} onKeyDown={event=>{if(event.key==='Escape'){setProjectPickerOpen(false);event.currentTarget.previousElementSibling instanceof HTMLElement && event.currentTarget.previousElementSibling.focus();}}}>
          <input type="search" aria-label="Find a project" placeholder="Find a project…" value={projectSearch} onChange={event=>setProjectSearch(event.target.value)}/>
          <nav aria-label="Projects">{view.projects.filter(project=>`${project.root} ${project.runtime??'codex'}`.toLowerCase().includes(projectSearch.toLowerCase())).map(project=><button type="button" key={`${project.runtime}:${project.root}`} aria-current={activeProject?.root===project.root&&(project.runtime??'codex')===runtime?'page':undefined} onClick={()=>{setRuntime(project.runtime??'codex');setSelectedRoot(project.root);setRoot(project.root);try { localStorage.setItem('corpstex.lastProject',project.root); } catch {} setProjectPickerOpen(false);setProjectSearch('');}}><strong>{project.root.split(/[\\/]/).filter(Boolean).at(-1)||project.root}</strong><small>{project.root}</small><span>{project.runtime??'codex'}{project.status==='ready'?'':project.status==='starting'?' · Connecting…':project.status==='failed'?' · Connection lost':' · Stopped'}</span></button>)}</nav>
          {!view.projects.some(project=>`${project.root} ${project.runtime??'codex'}`.toLowerCase().includes(projectSearch.toLowerCase())) && <p className="panel-note">{projectSearch?'No matching projects.':'No projects open.'}</p>}
          <button className="browse-project" type="button" onClick={()=>{setProjectPickerOpen(false);setBrowserOpen(true);}}>Browse folders…</button>
        <details className="project-settings" open={invalidRoot || Boolean(failure) || Boolean(restart) || undefined}>
        <summary>Connection details</summary>
        {runtimeProjects.length === 0 && <p className="panel-note">No projects open</p>}
        {activeProject && <div className="project-choice"><ProjectStatus project={activeProject}/>{activeProject.status==='failed' && <button type="button" onClick={()=>setRestart(activeProject)}>Review reconnect</button>}</div>}
        <form className="project-connect" noValidate onSubmit={event => { event.preventDefault(); void connect(); }}>
          <label htmlFor="project-root">Absolute project folder</label>
          <input ref={rootInput} id="project-root" value={root} onChange={event => { setRoot(event.target.value); setInvalidRoot(false); setFailure(''); }} required aria-invalid={invalidRoot} aria-describedby={failure ? 'project-connect-error' : undefined} spellCheck={false} autoComplete="off" placeholder="Absolute folder path" />
          {failure && <p id="project-connect-error" role="alert">{failure}</p>}
          {starting && <p role="status">Starting a project server…</p>}
          <button type="submit" disabled={connecting || starting}>{connecting ? 'Connecting…' : 'Connect project'}</button>
        </form>
        {restart && <div className="reconnect"><ProjectStatus project={restart}/><p>Reconnect starts a new server. Interrupted work will not resume automatically.</p><button type="button" onClick={()=>setRestart(null)}>Cancel reconnect</button><button type="button" disabled={connecting} onClick={()=>{void connect(restart);}}>Reconnect server</button></div>}
      </details></div>
      {activeProject&&activeProject.status!=='ready'&&<div className="project-recovery"><p role="status">{activeProject.status==='failed'?'Connection lost':activeProject.status==='starting'?'Connecting…':'Server stopped'}</p>{activeProject.status!=='starting'&&<button type="button" onClick={()=>{setProjectPickerOpen(true);setRestart(activeProject);}}>Review reconnect</button>}</div>}
      </>} />

      {pollFailure && <p className="global-error" role="alert">{pollFailure}</p>}

      <footer className="statusbar">
        <span>{selectedRoot || root || 'No project connected'}</span>
      </footer>
    {browserOpen && !view.exitRequested && <FolderBrowser initialPath={activeRoot} onClose={()=>setBrowserOpen(false)} onSelect={path=>{setRoot(path);setBrowserOpen(false);void connect(undefined,path);}} invoke={invoke} errorText={errorText}/> }
      </div>
      {showStartup && <Startup phase={startup === 'ready' ? 'welcome' : startup} error={typeof window === 'undefined' || (window as DesktopWindow).__TAURI__ ? 'The desktop status could not be read. Retry to check the workspace. No runtime connection is required.' : 'Open the CorpsTex desktop application to use your workspace.'} storageError={welcome.error} onRetry={()=>{setStartup('loading');setStartupAttempt(attempt=>attempt+1);}} onStart={finishWelcome}/>}
      {!showStartup && welcome.error && <p className="global-error" role="status">{welcome.error}</p>}
    </div>
    <ApprovalCenter enabled={preferences.showApprovalCenter} approvals={view.approvals} exiting={view.exitRequested} busyKeys={approvalBusy} invoke={approvalInvoke} errorText={errorText} />
    {view.exitRequested && <ShutdownDialog projects={view.exitProjects} onCancel={cancelExit} onConfirm={confirmExit} />}
  </>;
}

export { parsePreferences, parseModelChoice } from './preferences';
export { matchingApprovals } from './Approvals';

export { formatFileSize } from './FolderBrowser';

export { ActivityBar, activityState, activityPresets } from './ActivityBar';

export { composerToken, referencedSkills, composerCommands } from './ComposerInput';
export { quotaWindows, resetLabel, threadCost, weeklyWindow } from './UsageRail';

export { safeImageUrl, isLocalImageReference, readImageFiles, MessageImage } from './Media';
export { normalizeEvents, orderEvents, eventRow, historyRows, filterTimeline, timelineKey, TimelineView } from './Timeline';

export { ModelPicker, filterModels, recommendedFreeModels } from './ModelPicker';
export { RuntimeSettings, OpenCodeConnectionSettings } from './Settings';

export { parseSessionLayout, serializeSessionLayout, sessionKey, focusSession, closeSession } from './SessionWorkspace';
export { bindEditorInvoke, setEditorContext, grantEditorAccess, revokeEditorAccess, defaults as editorDefaults, sanitizeOptions, resolveOptions, fileUri, parseLanguageMappings, relocatedPath, manageFiles, restoreEditorSession, loadRecoveredFile, persistEditorSession, language, getEditor, openFile, updateFile, saveFile, syncFile, closeFile, discardFile, acceptMerge, acceptDisk, saveSettings, dirty as editorDirty } from './editor/state';
export { ThreadControls } from './ThreadControls';
export { RuntimeFields, RuntimeItemDetails, runtimeDetailData, TurnActivity } from './RuntimeDetails';
export { McpStatusView } from './McpStatus';

export { groupThreads, threadTime, ConversationLibrary } from './ConversationLibrary';
export { LspNotificationQueue } from './editor/lspQueue';
export { createDiagnostics, diagnosticsPrompt, isEditorCancellation } from './diagnostics';

export { parseEditorSession, serializeEditorSession, editorSessionKey } from './editor/recovery';

export { Startup, readWelcome, completeWelcome, welcomeKey } from './Startup';
