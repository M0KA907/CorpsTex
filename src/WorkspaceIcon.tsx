const paths={
  chat:'M4 4h16v12H9l-5 4V4Z',
  file:'M6 3h8l4 4v14H6V3Zm8 0v5h4',
  folder:'M3 6h7l2 2h9v12H3V6Z',
  search:'M16 16l5 5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z',
  filter:'M4 7h16M7 12h10M10 17h4',
  plus:'M12 5v14M5 12h14',
  refresh:'M20 4v6h-6M4 20v-6h6M5 8a8 8 0 0 1 13-3l2 3M4 16l2 3a8 8 0 0 0 13-3',
  access:'M7 11V7a5 5 0 0 1 10 0v4M5 11h14v10H5V11Zm7 4v3',
  arrow:'m14 5-7 7 7 7',
} as const;
export function WorkspaceIcon({name,size=16}:{name:keyof typeof paths;size?:number}) {
  return <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"><path d={paths[name]}/></svg>;
}
