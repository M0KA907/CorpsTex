import { useEffect, useRef, useState } from 'react';

type Server={name:string;authStatus:string|null;runtimeStatus:string|null;toolNames:string[];toolsUnavailable:boolean};
type Page={data:Server[];nextCursor:string|null};
type Project={root:string;generation:number;status:string};
type Invoke=<T>(command:string,args?:Record<string,unknown>)=>Promise<T>;

export function McpStatusView({servers}:{servers:Server[]}) {
  return <ul>{servers.map(server=><li key={server.name}><strong>{server.name}</strong><p>Connection: {server.runtimeStatus??'Not reported'} · Authentication: {server.authStatus??'Not reported'}</p>{server.toolsUnavailable&&<p role="status">Tool discovery failed. Refresh to retry.</p>}<details><summary>Tools ({server.toolNames.length})</summary><ul>{server.toolNames.map(name=><li key={name}>{name}</li>)}</ul></details></li>)}</ul>;
}

export function McpStatus({runtime,project,threadId,invoke,errorText}:{runtime:'codex'|'opencode';project:Project|null;threadId:string|null;invoke:Invoke;errorText:(error:unknown)=>string}) {
  const [servers,setServers]=useState<Server[]>([]);
  const [cursor,setCursor]=useState<string|null>(null);
  const [busy,setBusy]=useState(false);
  const [feedback,setFeedback]=useState('');
  const binding=JSON.stringify([runtime,project?.root,project?.generation,threadId]);
  const current=useRef(binding);
  current.current=binding;
  useEffect(()=>{current.current=binding;setServers([]);setCursor(null);setFeedback('');setBusy(false);return()=>{current.current='';};},[binding]);
  const ready=runtime==='codex'&&project?.status==='ready'&&Boolean(threadId);
  async function load(more=false) {
    if(!ready||busy)return;
    const key=binding;setBusy(true);setFeedback('');
    try {
      const page=await invoke<Page>('conversation_request',{runtime,root:project!.root,generation:project!.generation,action:{type:'listMcpStatus',threadId,cursor:more?cursor:null}});
      if(current.current!==key)return;
      setServers(old=>more?[...old.filter(server=>!page.data.some(next=>next.name===server.name)),...page.data]:page.data);
      setCursor(page.nextCursor);
      if(!page.data.length&&!more)setFeedback('Codex reported no MCP servers for this conversation.');
    }catch(error){if(current.current===key){setServers([]);setCursor(null);setFeedback(errorText(error));}}
    finally{if(current.current===key)setBusy(false);}
  }
  return <section className="settings-section" aria-label="MCP servers and tools"><h2>MCP servers and tools</h2>
    <p>Inspect the current conversation’s runtime connections and tool catalog. Configuration and sign-in remain managed by Codex.</p>
    {!ready&&<p>Select a connected Codex conversation to inspect its MCP servers.</p>}
    <button type="button" disabled={!ready||busy} onClick={()=>void load()}>{busy?'Reading catalog…':'Refresh MCP status'}</button>
    {feedback&&<p role="status">{feedback}</p>}
    <McpStatusView servers={servers}/>
    {cursor&&<button type="button" disabled={!ready||busy} onClick={()=>void load(true)}>More servers</button>}
  </section>;
}
