import { useEffect, useRef, useState, type ComponentProps } from 'react';
import { Conversation, type Thread } from './Conversation';
import './session-workspace.css';

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
  const [layout,setLayout]=useState(()=>{try{const raw=typeof window==='undefined'?null:localStorage.getItem(key);const initial=parseSessionLayout(raw);if(!raw){initial.tabs[0].runtime=props.runtime??'codex';initial.tabs[0].root=props.workspaceRoot;}return initial;}catch{const initial=empty();initial.tabs[0].runtime=props.runtime??'codex';initial.tabs[0].root=props.workspaceRoot;return initial;}});
  const [opened,setOpened]=useState<Record<string,number>>({});
  const [notice,setNotice]=useState('');
  const [titles,setTitles]=useState<Record<string,string>>({});
  const pending=useRef(new Set<string>());
  const host=useRef<HTMLDivElement>(null);
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
    const tab=layout.tabs.find(t=>t.id===id);if(!tab)return;
    if(tab.threadId)setOpened(old=>({...old,[id]:(old[id]??0)+1}));
    setLayout(old=>focusSession(old,id));props.onActivate(tab.runtime,tab.root);
  }
  function open(tab:SessionTab,thread?:Thread) {
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
  return <div className="session-workspace" ref={host}>
    <div className="session-toolbar">
      <div role="tablist" aria-label="Open sessions" className="session-tabs" onKeyDown={event=>{
        if(!['ArrowLeft','ArrowRight','Home','End','Delete'].includes(event.key))return;
        const target=event.target as HTMLElement;if(target.getAttribute('role')!=='tab')return;
        event.preventDefault();const i=layout.tabs.findIndex(t=>`session-tab-${t.id}`===target.id);
        if(event.key==='Delete'){close(layout.tabs[i]);return;}
        const index=event.key==='Home'?0:event.key==='End'?layout.tabs.length-1:(i+(event.key==='ArrowRight'?1:-1)+layout.tabs.length)%layout.tabs.length;
        focus(layout.tabs[index].id);document.getElementById(`session-tab-${layout.tabs[index].id}`)?.focus();
      }}>{layout.tabs.map(tab=><div className="session-tab" key={tab.id}><button type="button" role="tab" id={`session-tab-${tab.id}`} aria-controls={`session-panel-${tab.id}`} aria-selected={tab.id===layout.active} tabIndex={tab.id===layout.active?0:-1} title={`${tab.runtime} · ${tab.root} · ${tab.threadId??'No session'}`} disabled={props.exiting} onClick={()=>focus(tab.id)}>{titles[tab.id]||tab.threadId?.slice(0,8)||'New session'}<small>{tab.runtime} · {tab.root.split(/[\\/]/).filter(Boolean).at(-1)||'Choose project'}</small></button><button type="button" className="session-close" title="Close this view; runtime work and approvals continue" aria-label={`Close session ${titles[tab.id]||tab.threadId?.slice(0,8)||'New session'}`} disabled={props.exiting} onClick={()=>close(tab)}>×</button></div>)}</div>
      <div className="session-layout-controls"><button type="button" aria-label="New tab" title="New tab" disabled={props.exiting||layout.tabs.length>=limit} onClick={()=>open(active)}><span aria-hidden="true">+</span></button></div>
    </div>
    {notice&&<p className="session-notice" role="status">{notice}</p>}
    {layout.tabs.map(tab=>{
      const focused=tab.id===layout.active;const visible=focused;
      const project=props.workspaceProjects.find(p=>(p.runtime??'codex')===tab.runtime&&p.root===tab.root)??null;
      return <div key={tab.id} id={`session-panel-${tab.id}`} role="tabpanel" aria-labelledby={`session-tab-${tab.id}`} hidden={!visible} className={`session-panel ${focused?'session-focused':''}`} >
        <Conversation {...props} visible={visible} runtime={tab.runtime} project={project} sessionId={tab.id==='main'?'':tab.id} initialThread={tab.threadId} autoOpen={opened[tab.id]??0} onOpenSession={thread=>open(tab,thread)} onSessionState={(thread,busy)=>{
          if(busy)pending.current.add(tab.id);else pending.current.delete(tab.id);
          if(thread){const title=thread.name||thread.preview?.slice(0,48)||thread.id.slice(0,8);setTitles(old=>old[tab.id]===title?old:{...old,[tab.id]:title});setLayout(old=>old.tabs.some(t=>t.id===tab.id&&t.threadId!==thread.id)?{...old,tabs:old.tabs.map(t=>t.id===tab.id?{...t,threadId:thread.id}:t)}:old);}
        }} showTimeline={focused} onDraftState={undefined} controls={focused?props.controls:null} runtimeControls={focused?props.runtimeControls:null} settingsOpen={focused&&props.settingsOpen} connect={async()=>{
          if(!tab.root)return props.connect();
          await props.invoke('connect_project',{runtime:tab.runtime,root:tab.root,restart:null,acknowledgeLoss:false});return true;
        }}/>
      </div>;
    })}
  </div>;
}
