import { WorkspaceIcon } from './WorkspaceIcon';
import { useEffect, useRef, useState, type ComponentProps, type CSSProperties } from 'react';
import { Conversation, type Thread } from './Conversation';
import './session-workspace.css';
import { OpenTabs, useSidebarWidth, type OpenTab } from './WorkspaceTools';
import { parseTimelineMode, timelineModeKey, type TimelineMode } from './Timeline';
import { EditorSurface } from './editor/EditorSurface';
import { FileExplorer } from './editor/FileExplorer';
import { EditorChrome } from './editor/EditorChrome';
import { closeFile, dirty, setActiveFile, setEditorContext, setEditorSidebar, updateFile, useEditor } from './editor/state';
import './editor/editor.css';

type Runtime = 'codex' | 'opencode';
export type SessionTab = { id:string; runtime:Runtime; root:string; threadId?:string };
export type SessionLayout = { tabs:SessionTab[]; active:string };
const key = 'corpstex.workspace.v1';
const limit = 16;
const empty = ():SessionLayout => ({tabs:[{id:'main',runtime:'codex',root:''}],active:'main'});
export function sessionKey(tab:Pick<SessionTab,'runtime'|'root'|'threadId'>):string { return JSON.stringify([tab.runtime,tab.root,tab.threadId??null]); }
export function parseSessionLayout(raw:string|null):SessionLayout {
  try {
    if(!raw || raw.length>65536)return empty();
    const value=JSON.parse(raw);
    if(value.version!==1 || !Array.isArray(value.tabs))return empty();
    const tabs:SessionTab[]=[];
    for(const tab of value.tabs.slice(0,limit)) {
      if(!tab || typeof tab.id!=='string' || !/^[a-zA-Z0-9-]{1,80}$/.test(tab.id) || !['codex','opencode'].includes(tab.runtime) || typeof tab.root!=='string' || tab.root.length>4096 || (tab.root!=='' && !tab.root.startsWith('/') && !/^[a-zA-Z]:[\\/]/.test(tab.root) && !tab.root.startsWith('\\\\')) || (tab.threadId!==undefined && (typeof tab.threadId!=='string' || !tab.threadId || tab.threadId.length>512)))continue;
      const next:SessionTab={id:tab.id,runtime:tab.runtime,root:tab.root,...(tab.threadId?{threadId:tab.threadId}:{})};
      if(!tabs.some(old=>old.id===next.id || sessionKey(old)===sessionKey(next)))tabs.push(next);
    }
    if(!tabs.length)return empty();
    const active=tabs.some(t=>t.id===value.active)?value.active:tabs[0].id;
    return {tabs,active};
  } catch {return empty();}
}
export function serializeSessionLayout(layout:SessionLayout):string {
  return JSON.stringify({version:1,active:layout.active,tabs:layout.tabs.map(({id,runtime,root,threadId})=>({id,runtime,root,threadId}))});
}
export function focusSession(layout:SessionLayout,id:string):SessionLayout {
  if(!layout.tabs.some(t=>t.id===id))return layout;
  return {...layout,active:id};
}
export function closeSession(layout:SessionLayout,id:string):SessionLayout {
  const tabs=layout.tabs.filter(t=>t.id!==id);
  if(!tabs.length)return {...layout,tabs:[{id:'main',runtime:layout.tabs[0].runtime,root:layout.tabs[0].root}],active:'main'};
  return {...layout,tabs,active:layout.active===id?tabs[Math.max(0,layout.tabs.findIndex(t=>t.id===id)-1)].id:layout.active};
}

type Props=ComponentProps<typeof Conversation> & {workspaceRoot:string;workspaceProjects:NonNullable<ComponentProps<typeof Conversation>['project']>[];onActivate:(runtime:Runtime,root:string)=>void};
export function SessionWorkspace(props:Props) {
  const editor=useEditor();
  useEffect(()=>{void setEditorContext(props.workspaceRoot,props.runtime??'codex').catch(e=>setNotice(typeof e?.message==='string'?e.message:'Editor access could not be initialized.'));},[props.workspaceRoot,props.runtime]);
  const sidebarView=editor.sidebar;
  const [sidebarFlipping,setSidebarFlipping]=useState(false);
  function setSidebarView(view:'conversations'|'files') {
    if(view===sidebarView)return;
    const reduced=props.preferences.motion==='reduced'||window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    setSidebarFlipping(!reduced);setEditorSidebar(view);
    requestAnimationFrame(()=>host.current?.querySelector<HTMLButtonElement>(`${view==='files'?'.file-explorer':'.session-focused .projects'} .sidebar-navigation [aria-current=page]`)?.focus());
  }
  useEffect(()=>{
    if(!sidebarFlipping)return;
    const timer=setTimeout(()=>setSidebarFlipping(false),400);
    return()=>clearTimeout(timer);
  },[sidebarFlipping,sidebarView]);
  const activeFile=editor.files.find(f=>f.id===editor.active);
  useEffect(()=>{if(activeFile?.project)props.onActivate(activeFile.runtime,activeFile.project);},[activeFile?.id]);
  const [layout,setLayout]=useState(()=>{try{const raw=typeof window==='undefined'?null:localStorage.getItem(key);const initial=parseSessionLayout(raw);if(!raw){initial.tabs[0].runtime=props.runtime??'codex';initial.tabs[0].root=props.workspaceRoot;}return initial;}catch{const initial=empty();initial.tabs[0].runtime=props.runtime??'codex';initial.tabs[0].root=props.workspaceRoot;return initial;}});
  const [opened,setOpened]=useState<Record<string,number>>({});
  const [notice,setNotice]=useState('');
  const [timelineMode,setTimelineMode]=useState<TimelineMode>(()=>{
    try { return parseTimelineMode(typeof window === 'undefined' ? null : localStorage.getItem(timelineModeKey)); }
    catch { return 'collapsed'; }
  });
  function changeTimelineMode(mode:TimelineMode) {
    setTimelineMode(mode);
    if(mode==='expanded')setActiveFile(null);
    try { localStorage.setItem(timelineModeKey,mode); }
    catch { setNotice('Timeline visibility applies now, but could not be saved on this device.'); }
    // Settings retains focus; heading toggles focus the visible timeline.
    if(mode==='hidden'||host.current?.querySelector('.settings-dialog[open]'))return;
    requestAnimationFrame(()=>{
      host.current?.querySelector<HTMLElement>('.session-focused #timeline-title')?.focus();
    });
  }

  const [titles,setTitles]=useState<Record<string,string>>({});
  const pending=useRef(new Set<string>());
  const [working,setWorking]=useState<Record<string,boolean>>({});
  const [tabFinder,setTabFinder]=useState(false);
  const host=useRef<HTMLDivElement>(null);
  const sidebar=useSidebarWidth(sidebarView,host,setNotice);
  useEffect(()=>{const selected=host.current?.querySelector<HTMLElement>('.session-tabs [aria-selected="true"]');selected?.scrollIntoView({block:'nearest',inline:'nearest'});},[layout.active,editor.active]);
  const external=`${props.runtime}:${props.workspaceRoot}`;
  const previous=useRef(external);
  const activate=useRef(props.onActivate);activate.current=props.onActivate;
  useEffect(()=>{const tab=layout.tabs.find(t=>t.id===layout.active)!;if(tab.root)activate.current(tab.runtime,tab.root);},[]);
  useEffect(()=>{try{localStorage.setItem(key,serializeSessionLayout(layout));}catch{setNotice('Layout applies now, but could not be saved on this device.');}},[layout]);
  useEffect(()=>{
    if(previous.current===external)return;
    previous.current=external;
    const root=props.workspaceRoot;const runtime=props.runtime??'codex';
    setLayout(old=>{
      if(old.tabs.some(t=>t.id===old.active&&t.root===root&&t.runtime===runtime))return old;
      const existing=old.tabs.find(t=>t.root===root&&t.runtime===runtime);
      if(existing)return focusSession(old,existing.id);
      if(old.tabs.length>=limit){setNotice('Close a session tab before opening another project.');return old;}
      const blank=old.tabs.find(t=>!t.root&&!t.threadId);
      if(blank)return {...old,tabs:old.tabs.map(t=>t.id===blank.id?{...t,root,runtime}:t),active:blank.id};
      const tab={id:crypto.randomUUID(),root,runtime};
      return focusSession({...old,tabs:[...old.tabs.filter(t=>t.root||t.threadId),tab]},tab.id);
    });
  },[external]);
  function focus(id:string) {
    setActiveFile(null);
    const tab=layout.tabs.find(t=>t.id===id);if(!tab)return;
    if(tab.threadId)setOpened(old=>({...old,[id]:(old[id]??0)+1}));
    setLayout(old=>focusSession(old,id));props.onActivate(tab.runtime,tab.root);
  }
  function open(tab:SessionTab,thread?:Thread) {
    setActiveFile(null);
    const existing=thread && layout.tabs.find(t=>t.runtime===tab.runtime&&t.root===tab.root&&t.threadId===thread.id);
    if(existing){focus(existing.id);return;}
    if(thread && !tab.threadId && !pending.current.has(tab.id)){setLayout(old=>({...old,tabs:old.tabs.map(t=>t.id===tab.id?{...t,threadId:thread.id}:t)}));setOpened(old=>({...old,[tab.id]:(old[tab.id]??0)+1}));return;}
    if(layout.tabs.length>=limit){setNotice('Close a session tab before opening another.');return;}
    const next:SessionTab={id:crypto.randomUUID(),runtime:tab.runtime,root:tab.root,...(thread?{threadId:thread.id}:{})};
    setOpened(old=>({...old,[next.id]:1}));
    setLayout(old=>focusSession({...old,tabs:[...old.tabs,next]},next.id));
    props.onActivate(next.runtime,next.root);
  }
  function close(tab:SessionTab) {
    if(props.exiting)return;
    if(pending.current.has(tab.id) && !window.confirm('Close this tab and discard its unsent draft? In-flight runtime work continues; approvals remain in the Approval center.'))return;
    const next=closeSession(layout,tab.id);
    // A fresh empty tab must not reuse the mounted conversation being closed.
    if(layout.tabs.length===1)next.tabs[0].id=next.active=crypto.randomUUID();
    pending.current.delete(tab.id);setLayout(next);
    const active=next.tabs.find(t=>t.id===next.active)!;props.onActivate(active.runtime,active.root);
    requestAnimationFrame(()=>document.getElementById(`session-tab-${next.active}`)?.focus());
  }
  const active=layout.tabs.find(t=>t.id===layout.active)!;
  const sidebarNavigation=<nav className="sidebar-navigation" aria-label="Sidebar"><button type="button" aria-current={sidebarView==='conversations'?'page':undefined} onClick={()=>setSidebarView('conversations')}><WorkspaceIcon name="chat"/>Chats</button><button type="button" aria-label="Show project files" aria-current={sidebarView==='files'?'page':undefined} onClick={()=>setSidebarView('files')}><WorkspaceIcon name="folder"/>Files</button></nav>;
  const openTabs:OpenTab[]=[
    ...layout.tabs.map(tab=>({id:tab.id,kind:'Chats' as const,title:titles[tab.id]||tab.threadId?.slice(0,8)||'New session',project:tab.root,detail:tab.runtime,active:!activeFile&&tab.id===layout.active,working:working[tab.id],onOpen:()=>{focus(tab.id);requestAnimationFrame(()=>document.getElementById(`session-tab-${tab.id}`)?.focus());}})),
    ...editor.files.map(file=>({id:file.id,kind:'Files' as const,title:file.path,project:file.project,detail:`${file.runtime} · ${file.root}`,active:file.id===editor.active,unsaved:dirty(file),onOpen:()=>{setActiveFile(file.id);requestAnimationFrame(()=>document.getElementById(`file-tab-${file.id}`)?.focus());}})),
  ];
  return <div className="session-workspace" ref={host} style={{'--sidebar-width':`${sidebar.width}px`} as CSSProperties} data-sidebar-view={sidebarView} data-sidebar-flipping={sidebarFlipping} data-file-active={Boolean(activeFile)}>
    <FileExplorer runtime={props.runtime??'codex'} header={sidebarView==='files'?props.controls:null} navigation={sidebarNavigation} root={props.workspaceRoot} hidden={sidebarView!=='files'} onProject={root=>props.onActivate(props.runtime??'codex',root)} onNewConversation={()=>{setSidebarView('conversations');open(active);}}/>
    <EditorChrome/>
    {sidebar.resizer}
    {tabFinder&&<OpenTabs tabs={openTabs} onClose={()=>setTabFinder(false)}/>}
    <div className="session-toolbar">

      <div role="tablist" aria-label="Open sessions" className="session-tabs" data-has-files={editor.files.length>0} onKeyDown={event=>{
        if(!['ArrowLeft','ArrowRight','Home','End','Delete'].includes(event.key))return;
        const target=event.target as HTMLElement;if(target.getAttribute('role')!=='tab')return;
        event.preventDefault();
        const tabs=Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role=tab]'));
        const i=tabs.indexOf(target as HTMLButtonElement);
        if(event.key==='Delete'){const tab=layout.tabs.find(t=>`session-tab-${t.id}`===target.id);if(tab)close(tab);return;}
        const index=event.key==='Home'?0:event.key==='End'?tabs.length-1:(i+(event.key==='ArrowRight'?1:-1)+tabs.length)%tabs.length;
        tabs[index]?.click();tabs[index]?.focus();
      }}><div className="tab-group"><span className="tab-group-label">Chats <small>{layout.tabs.length}</small></span><div className="tab-group-items">{layout.tabs.map(tab=><div className="session-tab" key={tab.id}><button type="button" role="tab" id={`session-tab-${tab.id}`} aria-controls={`session-panel-${tab.id}`} aria-selected={!activeFile&&tab.id===layout.active} tabIndex={!activeFile&&tab.id===layout.active?0:-1} title={`${tab.runtime} · ${tab.root} · ${tab.threadId??'No session'}`} disabled={props.exiting} onClick={()=>focus(tab.id)}>{titles[tab.id]||tab.threadId?.slice(0,8)||'New session'}{working[tab.id]&&<span className="tab-working" aria-label="Working"> · Working</span>}<small>{tab.runtime} · {tab.root.split(/[\\/]/).filter(Boolean).at(-1)||'Choose project'}</small></button><button type="button" className="session-close" title="Close this view; runtime work and approvals continue" aria-label={`Close session ${titles[tab.id]||tab.threadId?.slice(0,8)||'New session'}`} disabled={props.exiting} onClick={()=>close(tab)}>×</button></div>)}</div></div>{editor.files.length>0&&<div className="tab-group"><span className="tab-group-label">Files <small>{editor.files.length}</small></span><div className="tab-group-items">{editor.files.map(file=><div className="session-tab file-tab" key={file.id} data-preview={file.preview}><button type="button" role="tab" id={`file-tab-${file.id}`} aria-controls={file.id===editor.active?'source-panel':`source-panel-${file.id}`} aria-selected={file.id===editor.active} tabIndex={file.id===editor.active?0:-1} title={file.root+'/'+file.path} onClick={()=>setActiveFile(file.id)} onDoubleClick={()=>updateFile(file.id,{preview:false})} onKeyDown={event=>{if(event.key==='Delete'){event.preventDefault();event.stopPropagation();closeFile(file.id);}}}>{dirty(file)?'● ':''}{file.conflict?'⚠ ':''}{file.path.split('/').at(-1)}<small>{file.root.split(/[\\/]/).filter(Boolean).at(-1)} · {file.preview?'Preview':'Source'}</small></button><button type="button" className="session-close" aria-label={`Close file ${file.path}`} disabled={file.saving||file.managing} onClick={()=>closeFile(file.id)}>×</button></div>)}</div></div>}</div>
      <div className="session-layout-controls"><button type="button" aria-label="Find open tab" title="Find open tab" aria-haspopup="dialog" onClick={()=>setTabFinder(true)}><svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7"><circle cx="10" cy="10" r="6"/><path d="m15 15 5 5"/></svg></button><button type="button" aria-label="New tab" title="New tab" disabled={props.exiting||layout.tabs.length>=limit} onClick={()=>open(active)}><span aria-hidden="true">+</span></button></div>
    </div>
    {editor.files.map(file=><div key={file.id} id={file.id===editor.active?'source-panel':`source-panel-${file.id}`} hidden={file.id!==editor.active} className="source-panel" role="tabpanel" aria-labelledby={`file-tab-${file.id}`}><EditorSurface file={file} visible={file.id===editor.active}/></div>)}
    {notice&&<p className="session-notice" role="status">{notice}</p>}
    {layout.tabs.map(tab=>{
      const focused=tab.id===layout.active;const visible=focused;
      const project=props.workspaceProjects.find(p=>(p.runtime??'codex')===tab.runtime&&p.root===tab.root)??null;
      return <div key={tab.id} id={`session-panel-${tab.id}`} role="tabpanel" aria-labelledby={`session-tab-${tab.id}`} hidden={!visible} className={`session-panel ${focused?'session-focused':''}`} >
        <Conversation {...props} sidebarHidden={sidebarView!=='conversations'} timelineMode={timelineMode} onTimelineModeChange={changeTimelineMode} editorRoot={activeFile?.root??props.workspaceRoot} sidebarToggle={sidebarNavigation} visible={visible&&!activeFile} runtime={tab.runtime} project={project} sessionId={tab.id==='main'?'':tab.id} initialThread={tab.threadId} autoOpen={opened[tab.id]??0} onOpenSession={thread=>open(tab,thread)} onSessionState={(thread,busy)=>{
          const running=Boolean(thread?.turns?.some(turn=>turn.status==='inProgress'));
          setWorking(old=>old[tab.id]===running?old:{...old,[tab.id]:running});
          if(busy)pending.current.add(tab.id);else pending.current.delete(tab.id);
          if(thread){const title=thread.name||thread.preview?.slice(0,48)||thread.id.slice(0,8);setTitles(old=>old[tab.id]===title?old:{...old,[tab.id]:title});setLayout(old=>old.tabs.some(t=>t.id===tab.id&&t.threadId!==thread.id)?{...old,tabs:old.tabs.map(t=>t.id===tab.id?{...t,threadId:thread.id}:t)}:old);}
        }} showTimeline={focused} onDraftState={undefined} controls={focused&&sidebarView==='conversations'?props.controls:null} runtimeControls={focused?props.runtimeControls:null} settingsOpen={focused&&props.settingsOpen} connect={async()=>{
          if(!tab.root)return props.connect();
          await props.invoke('connect_project',{runtime:tab.runtime,root:tab.root,restart:null,acknowledgeLoss:false});return true;
        }}/>
      </div>;
    })}
  </div>;
}
