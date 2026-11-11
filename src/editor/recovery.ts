import type { FileTab } from './state';

export type EditorView={selection:{startLineNumber:number;startColumn:number;endLineNumber:number;endColumn:number};scrollTop:number;scrollLeft:number};
export type EditorReference={root:string;path:string;project:string;runtime:'codex'|'opencode';preview:boolean;unsaved:boolean;cursor?:{lineNumber:number;column:number};view?:EditorView};
export type EditorSession={version:1;tabs:EditorReference[];active:number|null;sidebar:'conversations'|'files'};
export const editorSessionKey='corpstex.editor.session.v1';
const coordinate=(n:unknown):n is number=>typeof n==='number'&&Number.isSafeInteger(n)&&n>0&&n<=100000000;
const absolute=(s:unknown):s is string=>typeof s==='string'&&s.length>0&&s.length<=4096&&!/[\x00-\x1f\x7f]/.test(s)&&(s.startsWith('/')||/^[a-zA-Z]:[\\/]/.test(s)||s.startsWith('\\\\'));
function reference(value:unknown):EditorReference|null{
  if(!value||typeof value!=='object')return null;
  const v=value as Record<string,any>;
  if(!absolute(v.root)||!absolute(v.project)||!['codex','opencode'].includes(v.runtime)||typeof v.path!=='string'||!v.path||v.path.length>4096||v.path.split('/').some((p:string)=>!p||p==='.'||p==='..'||p.toLowerCase()==='.git'||p.startsWith('.corpstex-save-'))||/[\\:\x00-\x1f\x7f]/.test(v.path))return null;
  const ref:EditorReference={root:v.root,path:v.path,project:v.project,runtime:v.runtime,preview:v.preview===true,unsaved:v.unsaved===true};
  if(v.cursor&&coordinate(v.cursor.lineNumber)&&coordinate(v.cursor.column))ref.cursor={lineNumber:v.cursor.lineNumber,column:v.cursor.column};
  const s=v.view?.selection;
  if(s&&[s.startLineNumber,s.startColumn,s.endLineNumber,s.endColumn].every(coordinate)&&[v.view.scrollTop,v.view.scrollLeft].every(n=>typeof n==='number'&&Number.isFinite(n)&&n>=0&&n<=1000000000))ref.view={selection:{startLineNumber:s.startLineNumber,startColumn:s.startColumn,endLineNumber:s.endLineNumber,endColumn:s.endColumn},scrollTop:v.view.scrollTop,scrollLeft:v.view.scrollLeft};
  return ref;
}
export function parseEditorSession(raw:string|null):EditorSession|null{
  try{
    if(!raw||raw.length>256*1024)return null;
    const v=JSON.parse(raw);
    if(v.version!==1||!Array.isArray(v.tabs)||!v.tabs.length||v.tabs.length>32)return null;
    const tabs=v.tabs.map(reference);if(tabs.some((t:EditorReference|null)=>!t))return null;
    if(new Set(tabs.map((t:EditorReference)=>JSON.stringify([t.root,t.path]))).size!==tabs.length)return null;
    return {version:1,tabs,active:Number.isInteger(v.active)&&v.active>=0&&v.active<tabs.length?v.active:null,sidebar:v.sidebar==='files'?'files':'conversations'};
  }catch{return null;}
}
export function serializeEditorSession(files:FileTab[],active:string|null,sidebar:EditorSession['sidebar']):string{
  const tabs=files.map(f=>reference({root:f.root,path:f.path,project:f.project,runtime:f.runtime,preview:f.preview,unsaved:f.text!==f.disk.text||Boolean(f.conflict)||Boolean(f.recoveryUnsaved),cursor:f.cursor,view:f.view}));
  return JSON.stringify({version:1,tabs,active:active?files.findIndex(f=>f.id===active):null,sidebar});
}
