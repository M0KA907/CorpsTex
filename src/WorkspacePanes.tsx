import { useRef, type RefObject } from 'react';

export type PaneSide='left'|'right';
export type PaneLayout={left:string;right:string;focused:PaneSide;ratio:number};
export const paneLayoutKey='corpstex.workspace.panes.v1';
export function parsePaneLayout(raw:string|null,ids:string[],active:string):PaneLayout {
  const fallback:PaneLayout={left:active,right:'editor',focused:'left',ratio:50};
  try {
    const value=JSON.parse(raw??'null');
    if(!value||![value.left,value.right].every(id=>id==='editor'||ids.includes(id))||value.left===value.right)return fallback;
    return {left:value.left,right:value.right,focused:value.focused==='right'?'right':'left',ratio:typeof value.ratio==='number'&&Number.isFinite(value.ratio)?Math.max(25,Math.min(75,value.ratio)):50};
  } catch {return fallback;}
}
export function assignPane(layout:PaneLayout,side:PaneSide,content:string):PaneLayout {
  const other=side==='left'?'right':'left';
  return {...layout,[side]:content,[other]:layout[other]===content?layout[side]:layout[other],focused:side};
}
export function PaneDivider({host,ratio,onChange}:{host:RefObject<HTMLDivElement|null>;ratio:number;onChange:(ratio:number)=>void}) {
  const dragging=useRef(false);
  function change(value:number){onChange(Math.max(25,Math.min(75,value)));}
  return <div className="pane-divider" role="separator" aria-label="Resize workspace panes" aria-orientation="vertical" aria-valuemin={25} aria-valuemax={75} aria-valuenow={Math.round(ratio)} tabIndex={0}
    title="Drag to resize · Arrow keys to adjust · Enter to reset"
    onDoubleClick={()=>change(50)}
    onKeyDown={event=>{if(!['ArrowLeft','ArrowRight','Home','End','Enter'].includes(event.key))return;event.preventDefault();change(event.key==='Home'?25:event.key==='End'?75:event.key==='Enter'?50:ratio+(event.key==='ArrowLeft'?-2:2));}}
    onPointerDown={event=>{if(event.button!==0)return;event.preventDefault();event.currentTarget.focus();event.currentTarget.setPointerCapture(event.pointerId);dragging.current=true;}}
    onPointerMove={event=>{if(!dragging.current||!host.current)return;const left=host.current.querySelector<HTMLElement>('[data-pane=left] .workspace,[data-pane=left].editor-dock')?.getBoundingClientRect();const right=host.current.querySelector<HTMLElement>('[data-pane=right] .workspace,[data-pane=right].editor-dock')?.getBoundingClientRect();if(left&&right)change((event.clientX-left.left)/(right.right-left.left)*100);}}
    onPointerUp={event=>{dragging.current=false;event.currentTarget.releasePointerCapture(event.pointerId);}}
    onLostPointerCapture={()=>{dragging.current=false;}}/>;
}
