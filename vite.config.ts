import { defineConfig } from 'vite';
const sanitizerUrl=new URL('./node_modules/dompurify/dist/purify.es.mjs',import.meta.url);
const sanitizerPath=(sanitizerUrl.host?'//'+sanitizerUrl.host:'')+decodeURIComponent(sanitizerUrl.pathname).replace(/^\/([a-zA-Z]:\/)/,'$1');

export default defineConfig({
  // Monaco vendors DOMPurify as well as declaring an npm dependency. Route the
  // vendored import to the audited patch, in both dev and production builds.
  resolve: { alias: [{ find:'./dompurify/dompurify.js', replacement:sanitizerPath }] },
  optimizeDeps: { exclude:['monaco-editor'] },
  plugins: [{
    name:'corpstex-monaco-csp',
    transform(source,id) {
      if(!id.replace(/\\/g,'/').endsWith('/monaco-editor/esm/vs/base/browser/domStylesheets.js'))return;
      const marker="const style = document.createElement('style');";
      if(!source.includes(marker))throw new Error('Monaco stylesheet factory changed; review the CSP nonce integration before upgrading.');
      return source.replace(marker,marker+"\n    style.nonce = document.getElementById('corpstex-editor-style-nonce')?.nonce || ''; ");
    },
  }],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: { target: ['es2022', 'chrome105', 'safari15.4'] },
});
