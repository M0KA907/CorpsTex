import { useEffect, useRef, useState } from 'react';
import type * as Monaco from 'monaco-editor/editor/editor.api';
import { acceptDisk, acceptMerge, dirty, getEditor, language, message, notice, resolveOptions, saveFile, updateFile, useEditor, type FileTab } from './state';

const views=new Map<string,Monaco.editor.ICodeEditorViewState|null>();
export function pruneEditorViews(ids:Set<string>){for(const id of views.keys())if(!ids.has(id))views.delete(id);}
export function EditorSurface({file}:{file:FileTab}){
  const host=useRef<HTMLDivElement>(null);const editor=useRef<Monaco.editor.IStandaloneCodeEditor|null>(null);
  const {settings}=useEditor();const options=resolveOptions(settings,file.root);const current=useRef(file);current.current=file;
  const [loading,setLoading]=useState(true);const [failure,setFailure]=useState('');const [position,setPosition]=useState('Ln 1, Col 1');
  useEffect(()=>{
    let disposed=false;let cleanup=()=>{};setFailure('');setLoading(true);
    void import('./monaco').then(({monaco,modelFor})=>{
      if(disposed||!host.current)return;
      const options=resolveOptions(getEditor().settings,current.current.root);
      for(const [name,data] of Object.entries(getEditor().settings.themes))try{monaco.editor.defineTheme(name,data as Monaco.editor.IStandaloneThemeData);}catch{notice(`Theme ${name} is invalid.`);}
      const model=modelFor(current.current);if(model.getValue()!==current.current.text)model.setValue(current.current.text);
      let diff:Monaco.editor.IStandaloneDiffEditor|undefined;let original:Monaco.editor.ITextModel|undefined;
      const config={automaticLayout:true,theme:options.theme,fontSize:options.fontSize,fontFamily:options.fontFamily,fontLigatures:options.ligatures,minimap:{enabled:!file.large&&options.minimap},wordWrap:options.wrap?'on' as const:'off' as const,scrollBeyondLastLine:false,accessibilitySupport:'auto' as const,ariaLabel:`Source editor: ${file.path}`,renderWhitespace:'selection' as const,largeFileOptimizations:true};
      if(file.conflict){
        original=monaco.editor.createModel(file.conflict.text,language(file.path));diff=monaco.editor.createDiffEditor(host.current,{...config,originalEditable:false,renderSideBySide:window.innerWidth>=900,maxComputationTime:3000});diff.setModel({original,modified:model});editor.current=diff.getModifiedEditor();
      }else editor.current=monaco.editor.create(host.current,{...config,model});
      model.updateOptions({tabSize:options.tabSize,insertSpaces:options.insertSpaces});
      const code=editor.current;code.restoreViewState(views.get(file.id)??null);
      if(current.current.cursor){code.setPosition(current.current.cursor);code.revealPositionInCenter(current.current.cursor);}
      const change=model.onDidChangeContent(()=>{const f=current.current;if(model.getValue()!==f.text)updateFile(f.id,{text:model.getValue(),preview:false});});
      const cursor=code.onDidChangeCursorPosition(event=>{updateFile(file.id,{cursor:event.position});setPosition(`Ln ${event.position.lineNumber}, Col ${event.position.column}`);});
      code.addCommand(monaco.KeyMod.CtrlCmd|monaco.KeyCode.KeyS,()=>void saveFile(file.id));
      code.focus();setLoading(false);
      cleanup=()=>{views.set(file.id,code.saveViewState());change.dispose();cursor.dispose();if(diff)diff.dispose();else code.dispose();original?.dispose();editor.current=null;};
    }).catch(e=>{if(!disposed){setFailure(message(e));setLoading(false);}});
    return()=>{disposed=true;cleanup();};
  },[file.id,file.conflict?.revision]);
  useEffect(()=>{
    const code=editor.current;if(!code)return;
    code.updateOptions({fontSize:options.fontSize,fontFamily:options.fontFamily,fontLigatures:options.ligatures,minimap:{enabled:!file.large&&options.minimap},wordWrap:options.wrap?'on':'off'});
    code.getModel()?.updateOptions({tabSize:options.tabSize,insertSpaces:options.insertSpaces});
    void import('./monaco').then(({monaco})=>{
      for(const [name,data] of Object.entries(settings.themes))try{monaco.editor.defineTheme(name,data as Monaco.editor.IStandaloneThemeData);}catch{notice(`Theme ${name} is invalid.`);}
      monaco.editor.setTheme(options.theme);
    });
  },[JSON.stringify(options),settings.themes,file.id]);
  useEffect(()=>{
    const model=editor.current?.getModel();if(model&&model.getValue()!==file.text){const saved=editor.current?.saveViewState();model.setValue(file.text);editor.current?.restoreViewState(saved??null);}
  },[file.text]);
  useEffect(()=>{if(file.cursor&&editor.current&&getEditor().active===file.id){const pos=editor.current.getPosition();if(pos?.lineNumber!==file.cursor.lineNumber||pos.column!==file.cursor.column){editor.current.setPosition(file.cursor);editor.current.revealPositionInCenter(file.cursor);}}},[file.cursor]);
  return <section className="source-editor" aria-label={`Editing ${file.path}`}>
    <header className="editor-heading"><div><span className="editor-eyebrow">SOURCE</span><strong title={file.root+'/'+file.path}>{file.path}</strong></div><button type="button" disabled={!dirty(file)||file.saving||Boolean(file.conflict||file.missing)} onClick={()=>void saveFile(file.id)}>{file.saving?'Saving…':'Save · Ctrl+S'}</button></header>
    {file.conflict&&<div className="editor-conflict" role="alert"><strong>Disk changed — your buffer is preserved.</strong><p>Disk version on the left; editable merge result on the right. On narrow screens the diff is inline. Review changes before accepting.</p><button type="button" onClick={()=>acceptDisk(file.id)}>Load disk version</button><button type="button" onClick={()=>acceptMerge(file.id)}>Accept edited merge result</button></div>}
    {file.missing&&<p className="editor-conflict" role="alert">{file.missing} Buffer retained. Restore the file externally to continue saving, or copy its contents into a new file.</p>}
    {file.large&&<p className="editor-warning">Large file mode · LSP and minimap disabled · maximum 32 MiB</p>}
    {loading&&<p className="editor-warning" role="status">Loading editor…</p>}
    {failure?<p role="alert">{failure}</p>:<div className="monaco-host" ref={host}/>}
    <footer className="editor-status"><span>{dirty(file)?'● Unsaved':'Saved'}{file.preview?' · Preview':''}</span><span>{position}</span><span>{file.disk.encoding.toUpperCase()}{file.disk.bom?' + BOM':''} · {file.text.includes('\r\n')?'CRLF':'LF'} · {language(file.path)}</span></footer>
  </section>;
}
