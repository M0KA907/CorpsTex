import { useEffect, useRef, useState } from 'react';
import type * as Monaco from 'monaco-editor/editor/editor.api';
import { acceptDisk, acceptMerge, dirty, getEditor, language, loadRecoveredFile, message, notice, resolveOptions, saveFile, updateFile, useEditor, type FileTab } from './state';

const views=new Map<string,Monaco.editor.ICodeEditorViewState|null>();
export function pruneEditorViews(ids:Set<string>){for(const id of views.keys())if(!ids.has(id))views.delete(id);}
export function EditorSurface({file}:{file:FileTab}){
  if(file.unloaded)return <section className="source-editor" aria-label={`Recovered reference: ${file.path}`}>
    <header className="editor-heading"><strong>{file.path}</strong></header>
    <div className="editor-warning"><p>Restored tab reference only. No file contents or folder permissions were restored.</p>
      {file.recoveryUnsaved&&<p role="alert">This tab had unsaved edits. Their text was not stored and is unrecoverable.</p>}
      <p>Select Files and explicitly grant access to {file.root} for {file.runtime} · {file.project}. Then load the current saved file from disk.</p>
      {file.missing&&<p role="alert">{file.missing}</p>}
      <button type="button" disabled={file.saving} onClick={()=>void loadRecoveredFile(file.id)}>{file.saving?'Loading…':'Load current saved file'}</button>
    </div>
  </section>;
  return <LoadedSurface file={file}/>;
}
function LoadedSurface({file}:{file:FileTab}){
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
      let diff:Monaco.editor.IStandaloneDiffEditor|undefined;let original:Monaco.editor.ITextModel|undefined;let labelSubscriptions:Monaco.IDisposable[]=[];
      const config={readOnly:Boolean(current.current.managing),automaticLayout:true,theme:options.theme,fontSize:options.fontSize,fontFamily:options.fontFamily,fontLigatures:options.ligatures,minimap:{enabled:!file.large&&options.minimap},wordWrap:options.wrap?'on' as const:'off' as const,scrollBeyondLastLine:false,accessibilitySupport:'auto' as const,ariaLabel:`Source editor: ${file.path}`,renderWhitespace:'selection' as const,largeFileOptimizations:true};
      if(file.conflict){
        original=monaco.editor.createModel(file.conflict.text,language(file.path));diff=monaco.editor.createDiffEditor(host.current,{...config,originalAriaLabel:`Disk version: ${file.path}`,modifiedAriaLabel:`Source editor: ${file.path}`,originalEditable:false,renderSideBySide:window.innerWidth>=900,maxComputationTime:3000});diff.setModel({original,modified:model});editor.current=diff.getModifiedEditor();
        // Repair labels after Monaco finishes resetting a diff sub-editor's options.
        labelSubscriptions=([[diff.getOriginalEditor(),`Disk version: ${file.path}`],[diff.getModifiedEditor(),`Source editor: ${file.path}`]] as const).map(([target,ariaLabel])=>{const enforce=()=>queueMicrotask(()=>{if(!disposed&&target.getOption(monaco.editor.EditorOption.ariaLabel)!==ariaLabel)target.updateOptions({ariaLabel});});const subscription=target.onDidChangeConfiguration(event=>{if(event.hasChanged(monaco.editor.EditorOption.ariaLabel))enforce();});enforce();return subscription;});
      }else editor.current=monaco.editor.create(host.current,{...config,model});
      model.updateOptions({tabSize:options.tabSize,insertSpaces:options.insertSpaces});
      const code=editor.current;const savedView=views.get(file.id);code.restoreViewState(savedView??null);
      if(!savedView&&current.current.view){code.setSelection(current.current.view.selection);code.setScrollPosition({scrollTop:current.current.view.scrollTop,scrollLeft:current.current.view.scrollLeft});}
      else if(current.current.cursor&&!savedView){code.setPosition(current.current.cursor);code.revealPositionInCenter(current.current.cursor);}
      const change=model.onDidChangeContent(()=>{const f=current.current;if(model.getValue()!==f.text)updateFile(f.id,{text:model.getValue(),preview:false});});
      const captureView=()=>{const selection=code.getSelection();if(selection)updateFile(file.id,{view:{selection:{startLineNumber:selection.startLineNumber,startColumn:selection.startColumn,endLineNumber:selection.endLineNumber,endColumn:selection.endColumn},scrollTop:code.getScrollTop(),scrollLeft:code.getScrollLeft()}});};
      const selection=code.onDidChangeCursorSelection(captureView);const scroll=code.onDidScrollChange(captureView);
      const cursor=code.onDidChangeCursorPosition(event=>{updateFile(file.id,{cursor:event.position});setPosition(`Ln ${event.position.lineNumber}, Col ${event.position.column}`);});
      code.addCommand(monaco.KeyMod.CtrlCmd|monaco.KeyCode.KeyS,()=>void saveFile(file.id));
      if(document.activeElement?.getAttribute('role')!=='tab')code.focus();setLoading(false);
      cleanup=()=>{views.set(file.id,code.saveViewState());captureView();change.dispose();cursor.dispose();selection.dispose();scroll.dispose();for(const subscription of labelSubscriptions)subscription.dispose();if(diff)diff.dispose();else code.dispose();original?.dispose();editor.current=null;};
    }).catch(e=>{if(!disposed){setFailure(message(e));setLoading(false);}});
    return()=>{disposed=true;cleanup();};
  },[file.id,file.root,file.path,file.conflict?.revision]);
  useEffect(()=>{
    const code=editor.current;if(!code)return;
    code.updateOptions({readOnly:Boolean(file.managing),fontSize:options.fontSize,fontFamily:options.fontFamily,fontLigatures:options.ligatures,minimap:{enabled:!file.large&&options.minimap},wordWrap:options.wrap?'on':'off'});
    code.getModel()?.updateOptions({tabSize:options.tabSize,insertSpaces:options.insertSpaces});
    void import('./monaco').then(({monaco,refreshModelLanguage})=>{
      if(editor.current!==code)return;const model=code.getModel();if(model)refreshModelLanguage(model,file.path);
      for(const [name,data] of Object.entries(settings.themes))try{monaco.editor.defineTheme(name,data as Monaco.editor.IStandaloneThemeData);}catch{notice(`Theme ${name} is invalid.`);}
      monaco.editor.setTheme(options.theme);
    });
  },[JSON.stringify(options),settings.themes,settings.languageMappings,file.id,file.managing,file.large]);
  useEffect(()=>{
    const model=editor.current?.getModel();if(model&&model.getValue()!==file.text){const saved=editor.current?.saveViewState();model.setValue(file.text);editor.current?.restoreViewState(saved??null);}
  },[file.text]);
  useEffect(()=>{if(file.cursor&&editor.current&&getEditor().active===file.id){const pos=editor.current.getPosition();if(pos?.lineNumber!==file.cursor.lineNumber||pos.column!==file.cursor.column){editor.current.setPosition(file.cursor);editor.current.revealPositionInCenter(file.cursor);}}},[file.cursor]);
  return <section className="source-editor" aria-label={`Editing ${file.path}`}>
    <header className="editor-heading"><div><span className="editor-eyebrow">SOURCE</span><strong title={file.root+'/'+file.path}>{file.path}</strong></div><button type="button" disabled={!dirty(file)||file.saving||file.managing||Boolean(file.conflict||file.missing)} onClick={()=>void saveFile(file.id)}>{file.saving?'Saving…':'Save · Ctrl+S'}</button></header>
    {file.conflict&&<div className="editor-conflict" role="alert"><strong>Disk changed — your buffer is preserved.</strong><p>Disk version on the left; editable merge result on the right. On narrow screens the diff is inline. Review changes before accepting.</p><button type="button" onClick={()=>acceptDisk(file.id)}>Load disk version</button><button type="button" onClick={()=>acceptMerge(file.id)}>Accept edited merge result</button></div>}
    {file.missing&&<p className="editor-conflict" role="alert">{file.missing} Buffer retained. Restore the file externally to continue saving, or copy its contents into a new file.</p>}
    {file.recoveryUnsaved&&<p className="editor-warning">Previous unsaved edits were not stored and could not be recovered. This is the current saved file.</p>}
    {file.large&&<p className="editor-warning">Large file mode · LSP and minimap disabled · maximum 32 MiB</p>}
    {loading&&<p className="editor-warning" role="status">Loading editor…</p>}
    {failure?<p role="alert">{failure}</p>:<div className="monaco-host" ref={host}/>}
    <footer className="editor-status"><span>{dirty(file)?'● Unsaved':'Saved'}{file.preview?' · Preview':''}</span><span>{position}</span><span>{file.disk.encoding.toUpperCase()}{file.disk.bom?' + BOM':''} · {file.text.includes('\r\n')?'CRLF':'LF'} · {language(file.path)}</span></footer>
  </section>;
}
