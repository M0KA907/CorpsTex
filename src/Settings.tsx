import { EditorSettings } from './editor/EditorSettings';
import { ActivityBar, activityPresets } from './ActivityBar';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { defaults, type Preferences } from './preferences';
import type { TimelineMode } from './Timeline';

type Project = {root: string; generation: number; status: string; activeTurns?:number; pendingRequests?:number};
type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
type Config = {values: Record<string, unknown>; version: string | null; origins: Record<string,string>};
type Requirements = {requirements: {allowedApprovalPolicies?: unknown[] | null; allowedSandboxModes?: string[] | null; allowedWebSearchModes?: string[] | null} | null};
const controls = [
  {key:'approval_policy',label:'Approval policy',options:[['untrusted','Ask for untrusted commands'],['on-request','Codex requests approval'],['never','Never ask for approval']]},
  {key:'sandbox_mode',label:'Sandbox access',options:[['read-only','Read only'],['workspace-write','Write inside workspace'],['danger-full-access','Full access · no sandbox']]},
  {key:'web_search',label:'Web search',options:[['disabled','Disabled'],['cached','Cached results'],['live','Live search']]},
];
function display(value: unknown): string { return value == null ? 'Not explicitly set' : typeof value === 'string' ? value : 'Custom policy (preserved)'; }

export function OpenCodeConnectionSettings({project,invoke,errorText}:{project:Project|null;invoke:Invoke;errorText:(error:unknown)=>string}) {
  const [busy,setBusy]=useState(false);
  const [feedback,setFeedback]=useState('');
  const mounted=useRef(true);
  useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;};},[]);
  const canRefresh=project?.status==='ready'&&!project.activeTurns&&!project.pendingRequests;
  async function refresh() {
    if(!project||busy||!canRefresh)return;
    setBusy(true);setFeedback('');
    try {await invoke('reconnect_opencode',{root:project.root,generation:project.generation});if(mounted.current)setFeedback('OpenCode is reconnecting with its current provider settings.');}
    catch(error){if(mounted.current)setFeedback(errorText(error));}
    finally{if(mounted.current)setBusy(false);}
  }
  return <section className="settings-section" aria-labelledby="opencode-connection-title">
    <h2 id="opencode-connection-title">OpenCode connection</h2>
    <p>OpenCode manages provider accounts and credentials. CorpsTex uses your existing OpenCode configuration across app restarts.</p>
    <p>To connect or change a provider, run <code>opencode auth login</code> in your terminal and follow OpenCode’s instructions. CorpsTex does not collect or store provider credentials.</p>
    {project ? <>
      <p>After changing accounts, refresh the connection when all turns and approval requests have finished.</p>
      <button type="button" disabled={busy||!canRefresh} onClick={()=>void refresh()}>Refresh connection</button>
    </> : <p>Select OpenCode in the runtime picker and connect your project. If OpenCode is missing, install it separately and make it available on your desktop app’s PATH.</p>}
    {busy&&<p role="status">Waiting for OpenCode…</p>}
    {feedback&&<p role="status" className="settings-notice">{feedback}</p>}
    <p className="settings-caption">Free labels come from the model catalog. Your provider’s access rules, limits, and pricing apply.</p>
  </section>;
}

export function RuntimeSettings({runtime='codex', project, invoke, errorText}: {runtime?:'codex'|'opencode';project: Project | null; invoke: Invoke; errorText: (error: unknown)=>string}) {
  const [config, setConfig] = useState<Config | null>(null);
  const [requirements, setRequirements] = useState<Requirements | null>(null);
  const [edits, setEdits] = useState<Record<string,string>>({});
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState('');
  const [requirementsError, setRequirementsError] = useState('');
  const [review, setReview] = useState(false);
  const current = useRef('');
  const binding = `${project?.root}:${project?.generation}`; current.current = binding;
  const ready = project?.status === 'ready' && runtime==='codex';
  const changes = Object.entries(edits).filter(([key,value])=>value !== '' && value !== config?.values[key]);
  const securityChange = changes.some(([key])=>key==='approval_policy'||key==='sandbox_mode');
  async function request<T>(action: Record<string,unknown>) {
    return invoke<T>('conversation_request',{root:project!.root,generation:project!.generation,action});
  }
  async function load() {
    if (!ready) return;
    const key = binding;
    setBusy(true);setFeedback('');setReview(false);setRequirementsError('');
    const results = await Promise.allSettled([request<Config>({type:'readSettings'}),request<Requirements>({type:'readRequirements'})]);
    if (current.current !== key) return;
    const [settings, constraints] = results;
    if (settings.status === 'fulfilled') {setConfig(settings.value);setEdits({});}
    else {setConfig(null);setFeedback(errorText(settings.reason));}
    if (constraints.status === 'fulfilled') setRequirements(constraints.value);
    else {setRequirements(null);setRequirementsError('Managed requirements could not be read. Permission editing is unavailable until Refresh succeeds.');}
    setBusy(false);
  }
  useEffect(()=>{setConfig(null);setEdits({});setFeedback('');setReview(false);setBusy(false);if(ready) void load();},[binding,ready]);
  async function save() {
    if (!config?.version || !changes.length || busy || !review) return;
    const key = binding;
    setBusy(true);setFeedback('');
    try {
      const result = await request<{status:string}>({type:'writeSettings',expectedVersion:config.version,changes:changes.map(([key,value])=>({key,value}))});
      if (current.current !== key) return;
      setReview(false);setEdits({});setConfig(null);
      const updated = await request<Config>({type:'readSettings'});
      if (current.current !== key) return;
      setConfig(updated);
      setFeedback(result.status === 'okOverridden' ? 'Saved to user config, but a higher-priority layer overrides one or more values. Effective values below show what Codex resolves.' : 'Saved to Codex user config. New conversations use these defaults; existing threads and other running servers may keep their current settings.');
    } catch(error) {if(current.current===key) {setReview(false);setFeedback(`${errorText(error)} Refresh before retrying. If the write succeeded but verification failed, inspect the effective values before saving again.`);}}
    finally {if(current.current===key)setBusy(false);}
  }
  function allowed(key: string, value: string) {
    if (!requirements) return false;
    const r = requirements.requirements;
    const list = key==='approval_policy'?r?.allowedApprovalPolicies:key==='sandbox_mode'?r?.allowedSandboxModes:r?.allowedWebSearchModes;
    return !Array.isArray(list) || list.includes(value) || (key==='sandbox_mode'&&list.includes({'read-only':'readOnly','workspace-write':'workspaceWrite','danger-full-access':'dangerFullAccess'}[value] ?? value));
  }
  if(runtime==='opencode')return <>
    <OpenCodeConnectionSettings project={project} invoke={invoke} errorText={errorText}/>
    <section className="settings-section" aria-labelledby="runtime-settings-title"><h2 id="runtime-settings-title">OpenCode permissions</h2><p>Choose a model, effort, and session mode under Model & reasoning.</p><p>Codex sandbox and approval settings do not apply to OpenCode. Its own permissions govern execution; requests appear in the Approval center.</p></section>
  </>;
  return <section className="settings-section" aria-labelledby="runtime-settings-title">
    <div className="settings-section-heading"><div><h2 id="runtime-settings-title">Configuration & permissions</h2></div><button type="button" disabled={!ready||busy} onClick={()=>void load()}>Refresh config</button></div>
    <p>Effective configuration for <strong>{project?.root ?? 'your project'}</strong>. Saves update your Codex user config across clients, without reloading active turns.</p>
    {!ready && <p className="settings-notice">Connect a project to inspect and edit its Codex configuration.</p>}
    {busy && <p role="status">Checking with Codex…</p>}
    {feedback && <p role="status" className="settings-notice">{feedback}</p>}
    {requirementsError && <p role="alert">{requirementsError}</p>}
    {config && <>
      <dl className="effective-config">{['model','model_provider','model_reasoning_effort'].map(key=><div key={key}><dt>{key.replaceAll('_',' ')}</dt><dd>{display(config.values[key])}</dd></div>)}</dl>
      {controls.map(control=><label className="settings-field" key={control.key}><span>{control.label}<small>Effective: {display(config.values[control.key])} · {config.origins[control.key] ?? 'Codex default'}</small></span><select disabled={busy||!config.version||!requirements} value={edits[control.key] ?? ''} onChange={event=>{setEdits({...edits,[control.key]:event.target.value});setReview(false);}}><option value="">Keep current setting</option>{control.options.map(([value,label])=><option key={value} value={value} disabled={!allowed(control.key,value)}>{label}{!allowed(control.key,value)?' · managed restriction':''}</option>)}</select></label>)}
      {!config.version && <p role="alert">Codex did not report a writable user-config version. Editing is unavailable.</p>}
      {requirements?.requirements && <p className="settings-caption">Managed requirements are loaded. Codex validates every save.</p>}
      {securityChange && <p className="settings-notice">You are changing execution permissions for future conversations. “Full access” removes the sandbox; “Never ask” disables approval prompts. Existing pending approvals still require a response.</p>}
      {changes.length>0 && <div className="settings-review"><h3>Review user-config changes</h3><ul>{changes.map(([key,value])=><li key={key}><code>{key}</code>: {display(config.values[key])} → <strong>{value}</strong></li>)}</ul><label className="settings-confirm"><input type="checkbox" checked={review} onChange={event=>setReview(event.target.checked)} />Apply these user-wide defaults to future conversations.</label><button type="button" disabled={!review||busy||!config.version} onClick={()=>void save()}>Save Codex defaults</button></div>}
    </>}
  </section>;
}

export function Settings({onClose,preferences,onPreferences,storageError,modelControls,runtime,tools,editorRoot='',timelineMode='collapsed',onTimelineModeChange}: {onClose:()=>void;preferences:Preferences;onPreferences:(next:Preferences)=>void;storageError:string;modelControls:ReactNode;runtime:ReactNode;tools?:ReactNode;editorRoot?:string;timelineMode?:TimelineMode;onTimelineModeChange?:(mode:TimelineMode)=>void}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [section,setSection] = useState('appearance');
  useEffect(()=>{
    const node=dialog.current;
    const launcher=document.querySelector<HTMLElement>('[aria-label="Personal settings"]');
    node?.showModal();
    return()=>{node?.close();launcher?.focus();};
  },[]);
  const fields: {key:keyof Preferences;group:string;label:string;hint?:string;options:[string,string][]}[] = [
    {key:'palette',group:'Colors',label:'Palette',options:[['charcoal','Charcoal'],['midnight','Midnight'],['forest','Forest']]},
    {key:'accent',group:'Colors',label:'Accent',options:[['gold','Warm gold'],['mint','Fresh mint'],['blue','Sky blue']]},
    {key:'textSize',group:'Typography',label:'Text size',options:[['small','Small'],['medium','Medium'],['large','Large']]},
    {key:'width',group:'Layout',label:'Reading width',options:[['focused','Focused'],['wide','Wide']]},
    {key:'density',group:'Layout',label:'Spacing',options:[['comfortable','Comfortable'],['compact','Compact']]},
    {key:'background',group:'Effects',label:'Background dots',options:[['animated','Animated'],['static','Static'],['off','Off']]},
    {key:'texture',group:'Effects',label:'Grain texture',options:[['on','On'],['off','Off']]},
    {key:'surfaces',group:'Effects',label:'Surfaces',hint:'Solid removes glass and blur, covering background dots. Grain texture stays independent.',options:[['glass','Glass'],['solid','Solid']]},
    {key:'motion',group:'Effects',label:'Motion',options:[['system','Lively · respect system preference'],['reduced','Reduced']]},
  ];
  return <dialog className="settings-dialog" ref={dialog} aria-labelledby="settings-title" onCancel={event=>{event.preventDefault();onClose();}} onClose={event=>{if(!event.currentTarget.open)onClose();}}>
    <header className="settings-header"><div><h1 id="settings-title">Settings</h1></div><button type="button" autoFocus onClick={onClose} aria-label="Close settings">Close <span aria-hidden="true">×</span></button></header>
    <nav className="settings-nav" aria-label="Settings categories">{[['appearance','Appearance'],['activity','Thinking bar'],['models','Model & reasoning'],['editor','Editor'],['tools','MCP tools'],['runtime','Runtime & permissions']].map(([id,label])=><button type="button" key={id} aria-current={section===id?'page':undefined} onClick={()=>setSection(id)}>{label}</button>)}</nav>
    <div className="settings-body" key={section} tabIndex={0} role="region" aria-label="Settings content">
      {section==='appearance' && <section className="settings-section" aria-labelledby="appearance-title"><h2 id="appearance-title">Appearance</h2><p>Saved on this device.</p><div className="palette-preview" aria-hidden="true"><span>c/</span><div><i/><i/><i/></div></div>{['Colors','Typography','Layout','Effects'].map(group=><fieldset className="appearance-group" key={group}><legend>{group}</legend>{fields.filter(field=>field.group===group).map(field=><label className="settings-field" key={field.key}><span>{field.label}{field.hint&&<small id={`appearance-${field.key}-hint`}>{field.hint}</small>}</span><select value={String(preferences[field.key])} aria-describedby={field.hint?`appearance-${field.key}-hint`:undefined} onChange={event=>onPreferences({...preferences,[field.key]:event.target.value})}>{field.options.map(([value,label])=><option key={value} value={value}>{label}</option>)}</select></label>)}</fieldset>)}<p className="settings-caption">Changes apply immediately. Reduced motion stops background animation and respects your system preference.</p><button type="button" onClick={()=>onPreferences({...defaults,activityPreset:preferences.activityPreset,activityDisplay:preferences.activityDisplay})}>Reset appearance</button>{storageError&&<p role="alert">{storageError}</p>}</section>}
      {section==='appearance' && <section className="settings-section"><label className="settings-field"><span>Show Approval Center Button<small>Controls the button and Ctrl+A shortcut. Inline approvals remain available.</small></span><input type="checkbox" checked={preferences.showApprovalCenter} onChange={event=>onPreferences({...preferences,showApprovalCenter:event.target.checked})}/></label></section>}
      {section==='appearance' && onTimelineModeChange && <section className="settings-section" aria-labelledby="timeline-settings-title"><h2 id="timeline-settings-title">Execution timeline</h2><p role="status">{timelineMode==='hidden'?'Hidden. Show it here when needed.':'Visible. Expand or collapse it from its heading.'} Visibility is saved on this device.</p><button type="button" className="timeline-visibility" onClick={()=>onTimelineModeChange(timelineMode==='hidden'?'expanded':'hidden')}>{timelineMode==='hidden'?'Show timeline':'Hide timeline'}</button></section>}
      {section==='activity' && <section className="settings-section" aria-labelledby="activity-settings-title"><h2 id="activity-settings-title">Thinking bar</h2><label className="settings-field"><span>Show activity as</span><select value={preferences.activityDisplay} onChange={event=>onPreferences({...preferences,activityDisplay:event.target.value as Preferences['activityDisplay']})}><option value="bar">Full bar</option><option value="compact">Compact</option><option value="hidden">Hidden</option></select></label><div className="activity-preview"><p className="settings-caption">Preview</p><ActivityBar preset={preferences.activityPreset} label="Thinking" active preview compact={preferences.activityDisplay==='compact'}/></div><div className="activity-presets" role="group" aria-label="Thinking bar styles">{activityPresets.map(([id,name])=><button type="button" key={id} aria-pressed={preferences.activityPreset===id} onClick={()=>onPreferences({...preferences,activityPreset:id})}><ActivityBar preset={id} label={name} active preview compact/></button>)}</div>{storageError&&<p role="alert">{storageError}</p>}</section>}
      {section==='models'  && <section className="settings-section" aria-labelledby="models-settings-title"><h2 id="models-settings-title">Model & reasoning</h2><p>Saved per project. Changes apply to the next turn.</p>{modelControls}</section>}
      {section==='runtime' && runtime}
      {section==='tools' && tools}
      {section==='editor' && <EditorSettings root={editorRoot}/>}
    </div>
  </dialog>;
}
