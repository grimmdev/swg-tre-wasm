import init, {
  TreArchiveHandle,
  list_files,
  extract_file,
  get_tre_file_count,
  create_demo_tre,
  init_panic_hook
} from './pkg/swg_tre_wasm.js';

// Application State
let wasmReady = false;
let currentHandle = null;
let currentEntries = [];
let activeEntry = null;
let activeRawData = null;

// Filters
let activeDirectory = 'all';
let activeTypeFilter = 'all';
let searchQuery = '';
let currentViewMode = 'hex'; // 'hex' | 'text'

// DOM Elements
const archiveNamePill = document.getElementById('archive-name-pill');
const btnOpenTre = document.getElementById('btn-open-tre');
const btnLoadDemo = document.getElementById('btn-load-demo');
const btnExportCsv = document.getElementById('btn-export-csv');
const fileInput = document.getElementById('file-input');

const treeList = document.getElementById('tree-list');
const folderCountBadge = document.getElementById('folder-count-badge');
const treeCountAll = document.getElementById('tree-count-all');

const searchInput = document.getElementById('search-input');
const typeFilterGroup = document.getElementById('type-filter-group');
const tableBody = document.getElementById('table-body');

// Inspector DOM
const inspExtPill = document.getElementById('insp-ext-pill');
const propFilename = document.getElementById('prop-filename');
const propFilepath = document.getElementById('prop-filepath');
const propSize = document.getElementById('prop-size');
const propPacked = document.getElementById('prop-packed');
const propCompression = document.getElementById('prop-compression');
const propCrc32 = document.getElementById('prop-crc32');
const propOffset = document.getElementById('prop-offset');
const btnExtractSelected = document.getElementById('btn-extract-selected');
const hexDisplay = document.getElementById('hex-display');
const btnViewHex = document.getElementById('btn-view-hex');
const btnViewText = document.getElementById('btn-view-text');

// Status Bar DOM
const statusMsg = document.getElementById('status-msg');
const statusRecords = document.getElementById('status-records');
const statusUnpackedSize = document.getElementById('status-unpacked-size');
const statusTiming = document.getElementById('status-timing');

// Utility Functions
function normalizePath(p) {
  return p.replace(/\\/g, '/');
}

function getBasename(path) {
  const normalized = normalizePath(path);
  const parts = normalized.split('/');
  return parts.pop() || 'unnamed_asset';
}

function getExtension(path) {
  const base = getBasename(path);
  const idx = base.lastIndexOf('.');
  return idx > 0 ? base.substring(idx + 1).toLowerCase() : '';
}

function getParentDirectory(path) {
  const normalized = normalizePath(path);
  const idx = normalized.lastIndexOf('/');
  return idx > 0 ? normalized.substring(0, idx) : '';
}

function formatBytes(bytes) {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

function getCompressionLabel(code) {
  switch (code) {
    case 0: return 'None';
    case 2: return 'Zlib (Deflate)';
    default: return `Method #${code}`;
  }
}

// Download Asset With Correct Filename and Extension
async function downloadExtractedFile(data, originalPath) {
  const basename = getBasename(originalPath);
  const ext = getExtension(basename);

  // Create an independent copy of WASM bytes so the buffer is detached-safe
  const copy = new Uint8Array(data.length);
  copy.set(data);

  // Modern File System Access API for Chrome/Edge (opens native Save As dialog with filename & extension)
  if (window.showSaveFilePicker) {
    try {
      const handle = await window.showSaveFilePicker({
        suggestedName: basename,
        types: [{
          description: ext ? `${ext.toUpperCase()} Asset` : 'Asset File',
          accept: { 'application/octet-stream': ext ? [`.${ext}`] : [] }
        }]
      });
      const writable = await handle.createWritable();
      await writable.write(copy);
      await writable.close();
      statusMsg.textContent = `Saved ${basename}`;
      return;
    } catch (err) {
      if (err.name === 'AbortError') {
        return; // User cancelled
      }
      console.warn('showSaveFilePicker fallback:', err);
    }
  }

  // Standard <a> download fallback with 60s blob lifetime
  const blob = new Blob([copy], { type: 'application/octet-stream' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.style.display = 'none';
  a.href = url;
  a.download = basename;
  a.setAttribute('download', basename);
  document.body.appendChild(a);
  a.click();

  setTimeout(() => {
    try {
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    } catch (e) {}
  }, 60000);
}

// Initialize WASM
async function initWasm() {
  try {
    statusMsg.textContent = 'Initializing WASM module...';
    await init();
    init_panic_hook();
    wasmReady = true;
    statusMsg.textContent = 'Engine ready';
  } catch (err) {
    console.error('WASM initialization error:', err);
    statusMsg.textContent = 'WASM Init Error';
  }
}

// Load and Parse Archive
async function loadArchiveData(bytes, archiveName) {
  if (!wasmReady) return;

  statusMsg.textContent = `Parsing ${archiveName}...`;
  const t0 = performance.now();

  try {
    currentHandle = new TreArchiveHandle(bytes);
    const summary = currentHandle.get_summary();
    const duration = (performance.now() - t0).toFixed(1);

    archiveNamePill.textContent = `${archiveName} (TREE:0005)`;
    statusMsg.textContent = `Archive loaded (${summary.total_files.toLocaleString()} items)`;
    statusRecords.textContent = `${summary.total_files.toLocaleString()} items`;
    statusUnpackedSize.textContent = `${formatBytes(summary.decompressed_size)} total`;
    statusTiming.textContent = `Parse: ${duration}ms`;

    btnExportCsv.disabled = false;

    // Fetch catalog
    currentEntries = currentHandle.get_all_entries();

    buildDirectoryTree();
    renderTable();
    clearInspector();

  } catch (err) {
    statusMsg.textContent = 'Parse Failed';
    alert(`Failed to parse TRE archive: ${err}`);
  }
}

// Build Sidebar Directory Tree
function buildDirectoryTree() {
  const dirMap = new Map();
  dirMap.set('all', currentEntries.length);

  for (const entry of currentEntries) {
    const norm = normalizePath(entry.name);
    const parts = norm.split('/');
    if (parts.length > 1) {
      let currentPath = '';
      for (let i = 0; i < parts.length - 1; i++) {
        currentPath = currentPath ? `${currentPath}/${parts[i]}` : parts[i];
        dirMap.set(currentPath, (dirMap.get(currentPath) || 0) + 1);
      }
    }
  }

  folderCountBadge.textContent = (dirMap.size - 1).toString();
  treeCountAll.textContent = currentEntries.length.toLocaleString();

  // Render tree items
  const sortedDirs = Array.from(dirMap.keys())
    .filter(d => d !== 'all')
    .sort();

  const itemsHtml = [
    `<div class="tree-item ${activeDirectory === 'all' ? 'active' : ''}" data-folder="all">
      <span class="tree-item-name">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path></svg>
        All Files
      </span>
      <span class="tree-count">${currentEntries.length.toLocaleString()}</span>
    </div>`
  ];

  for (const dir of sortedDirs) {
    const depth = dir.split('/').length;
    const paddingLeft = 12 + (depth - 1) * 12;
    const count = dirMap.get(dir);
    const displayName = dir.split('/').pop() + '/';

    itemsHtml.push(`
      <div class="tree-item ${activeDirectory === dir ? 'active' : ''}" data-folder="${dir}" style="padding-left: ${paddingLeft}px;">
        <span class="tree-item-name">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path></svg>
          ${displayName}
        </span>
        <span class="tree-count">${count.toLocaleString()}</span>
      </div>
    `);
  }

  treeList.innerHTML = itemsHtml.join('');
}

// Filter and Render Table
function renderTable() {
  if (!currentEntries || currentEntries.length === 0) {
    tableBody.innerHTML = `<tr><td colspan="8" style="text-align: center; color: var(--text-muted); padding: 40px 0;">No records found.</td></tr>`;
    return;
  }

  const query = searchQuery.trim().toLowerCase();
  const typeTokens = activeTypeFilter === 'all' ? null : activeTypeFilter.split(',');

  const filtered = currentEntries.filter(entry => {
    const norm = normalizePath(entry.name);

    // Directory filter
    if (activeDirectory !== 'all') {
      if (!norm.startsWith(activeDirectory + '/')) return false;
    }

    // Type filter
    if (typeTokens) {
      const ext = getExtension(norm);
      if (!typeTokens.includes(ext)) return false;
    }

    // Search query
    if (query && !norm.toLowerCase().includes(query)) {
      return false;
    }

    return true;
  });

  statusRecords.textContent = `${filtered.length.toLocaleString()} matching of ${currentEntries.length.toLocaleString()}`;

  if (filtered.length === 0) {
    tableBody.innerHTML = `<tr><td colspan="8" style="text-align: center; color: var(--text-muted); padding: 40px 0;">No matching files in this directory.</td></tr>`;
    return;
  }

  const limit = Math.min(filtered.length, 500);
  const rows = [];

  for (let i = 0; i < limit; i++) {
    const item = filtered[i];
    const basename = getBasename(item.name);
    const ext = getExtension(basename) || 'bin';
    const isSelected = activeEntry && activeEntry.name === item.name;

    const ratio = item.uncompressed_size > 0
      ? ((1 - (item.compressed_size / item.uncompressed_size)) * 100).toFixed(0) + '%'
      : '0%';

    const crcHex = `0x${item.crc32.toString(16).padStart(8, '0').toUpperCase()}`;

    rows.push(`
      <tr class="${isSelected ? 'selected' : ''}" data-idx="${item.index}" data-name="${item.name}">
        <td class="col-idx">${item.index + 1}</td>
        <td class="col-name" title="${item.name}">${normalizePath(item.name)}</td>
        <td class="col-ext"><span class="badge-ext badge-${ext}">${ext}</span></td>
        <td class="col-size">${formatBytes(item.uncompressed_size)}</td>
        <td class="col-packed">${formatBytes(item.compressed_size)}</td>
        <td class="col-ratio">${ratio}</td>
        <td class="col-crc">${crcHex}</td>
        <td class="col-act">
          <button class="btn btn-sm btn-table-extract" data-name="${item.name}">Save</button>
        </td>
      </tr>
    `);
  }

  if (filtered.length > limit) {
    rows.push(`
      <tr>
        <td colspan="8" style="text-align: center; color: var(--text-muted); padding: 8px 0; font-size: 11px;">
          Showing first 500 of ${filtered.length.toLocaleString()} matching records.
        </td>
      </tr>
    `);
  }

  tableBody.innerHTML = rows.join('');
}

// Select Asset and Inspect
function selectAsset(filename) {
  const entry = currentEntries.find(e => e.name === filename);
  if (!entry) return;

  activeEntry = entry;
  activeRawData = null;

  const basename = getBasename(entry.name);
  const ext = getExtension(basename) || 'bin';

  inspExtPill.style.display = 'inline-block';
  inspExtPill.className = `badge-ext badge-${ext}`;
  inspExtPill.textContent = ext;

  propFilename.textContent = basename;
  propFilepath.textContent = normalizePath(entry.name);
  propSize.textContent = `${formatBytes(entry.uncompressed_size)} (${entry.uncompressed_size.toLocaleString()} B)`;
  propPacked.textContent = `${formatBytes(entry.compressed_size)} (${entry.compressed_size.toLocaleString()} B)`;
  propCompression.textContent = getCompressionLabel(entry.compression_method);
  propCrc32.textContent = `0x${entry.crc32.toString(16).padStart(8, '0').toUpperCase()}`;
  propOffset.textContent = `0x${(entry.data_start || 0).toString(16).toUpperCase()}`;

  btnExtractSelected.disabled = false;
  btnExtractSelected.innerHTML = `
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path><polyline points="7 10 12 15 17 10"></polyline><line x1="12" y1="15" x2="12" y2="3"></line></svg>
    Extract "${basename}"
  `;

  hexDisplay.textContent = 'Decompressing preview via WASM...';

  // Automatically decompress preview
  try {
    const t0 = performance.now();
    activeRawData = currentHandle.extract_file(entry.name);
    const extractMs = (performance.now() - t0).toFixed(2);
    statusTiming.textContent = `Extract: ${extractMs}ms`;
    renderInspectorContent();
  } catch (err) {
    hexDisplay.textContent = `Decompression Error: ${err}`;
  }

  // Highlight row in table
  tableBody.querySelectorAll('tr').forEach(r => {
    if (r.getAttribute('data-name') === filename) {
      r.classList.add('selected');
    } else {
      r.classList.remove('selected');
    }
  });
}

function clearInspector() {
  activeEntry = null;
  activeRawData = null;
  inspExtPill.style.display = 'none';
  propFilename.textContent = '-';
  propFilepath.textContent = '-';
  propSize.textContent = '-';
  propPacked.textContent = '-';
  propCompression.textContent = '-';
  propCrc32.textContent = '-';
  propOffset.textContent = '-';
  btnExtractSelected.disabled = true;
  btnExtractSelected.innerHTML = `
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path><polyline points="7 10 12 15 17 10"></polyline><line x1="12" y1="15" x2="12" y2="3"></line></svg>
    Extract & Save File
  `;
  hexDisplay.textContent = 'Select a file to inspect raw bytes...';
}

// Render Hex vs Text in Inspector
function renderInspectorContent() {
  if (!activeRawData || activeRawData.length === 0) {
    hexDisplay.textContent = '(Empty 0-byte file)';
    return;
  }

  if (currentViewMode === 'text') {
    const decoder = new TextDecoder('utf-8', { fatal: false });
    const text = decoder.decode(activeRawData.slice(0, 4096));
    hexDisplay.textContent = text;
  } else {
    // Monospaced 16-byte Hex Dump with Offset Gutter
    const maxBytes = Math.min(activeRawData.length, 512);
    let output = '';
    for (let offset = 0; offset < maxBytes; offset += 16) {
      const chunk = activeRawData.slice(offset, offset + 16);
      const offsetHex = offset.toString(16).padStart(8, '0');
      
      let hexParts = [];
      let asciiParts = '';
      
      for (let j = 0; j < 16; j++) {
        if (j < chunk.length) {
          const byte = chunk[j];
          hexParts.push(byte.toString(16).padStart(2, '0'));
          asciiParts += (byte >= 32 && byte <= 126) ? String.fromCharCode(byte) : '.';
        } else {
          hexParts.push('  ');
        }
      }

      // Add middle spacer at 8 bytes
      const hexLine = hexParts.slice(0, 8).join(' ') + '  ' + hexParts.slice(8).join(' ');
      output += `${offsetHex}:  ${hexLine}  |${asciiParts}|\n`;
    }

    if (activeRawData.length > maxBytes) {
      output += `\n... [${(activeRawData.length - maxBytes).toLocaleString()} more bytes in file]`;
    }

    hexDisplay.textContent = output;
  }
}

// Export CSV
function exportCatalogCsv() {
  if (!currentEntries || currentEntries.length === 0) return;

  const headers = ['Index', 'Path', 'UncompressedBytes', 'CompressedBytes', 'CRC32', 'CompressionMethod', 'Offset'];
  const rows = currentEntries.map(e => [
    e.index + 1,
    `"${normalizePath(e.name)}"`,
    e.uncompressed_size,
    e.compressed_size,
    `"0x${e.crc32.toString(16).toUpperCase()}"`,
    e.compression_method,
    e.data_start || 0
  ]);

  const csv = [headers.join(','), ...rows.map(r => r.join(','))].join('\n');
  const blob = new Blob([csv], { type: 'text/csv;charset=utf-8;' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `swg_archive_catalog.csv`;
  document.body.appendChild(a);
  a.click();
  setTimeout(() => {
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }, 100);
}

// Event Listeners
btnOpenTre.addEventListener('click', () => fileInput.click());

fileInput.addEventListener('change', async (e) => {
  const file = e.target.files[0];
  if (!file) return;
  const buffer = await file.arrayBuffer();
  loadArchiveData(new Uint8Array(buffer), file.name);
});

btnLoadDemo.addEventListener('click', async () => {
  if (!wasmReady) return;
  try {
    const res = await fetch('assets/demo_archive.tre');
    if (res.ok) {
      const buffer = await res.arrayBuffer();
      loadArchiveData(new Uint8Array(buffer), 'demo_archive.tre');
      return;
    }
  } catch (err) {
    console.warn('Could not fetch assets/demo_archive.tre, using in-memory generator:', err);
  }

  const demoBytes = create_demo_tre();
  loadArchiveData(demoBytes, 'demo_archive.tre');
});

btnExportCsv.addEventListener('click', exportCatalogCsv);

// Directory Tree Selection
treeList.addEventListener('click', (e) => {
  const item = e.target.closest('.tree-item');
  if (!item) return;

  treeList.querySelectorAll('.tree-item').forEach(i => i.classList.remove('active'));
  item.classList.add('active');

  activeDirectory = item.getAttribute('data-folder');
  renderTable();
});

// Type Filter Chips
typeFilterGroup.addEventListener('click', (e) => {
  const chip = e.target.closest('.filter-chip');
  if (!chip) return;

  typeFilterGroup.querySelectorAll('.filter-chip').forEach(c => c.classList.remove('active'));
  chip.classList.add('active');

  activeTypeFilter = chip.getAttribute('data-ext');
  renderTable();
});

// Search
searchInput.addEventListener('input', (e) => {
  searchQuery = e.target.value;
  renderTable();
});

// Table Click (Row Select or Action)
tableBody.addEventListener('click', (e) => {
  const saveBtn = e.target.closest('.btn-table-extract');
  if (saveBtn) {
    const filename = saveBtn.getAttribute('data-name');
    selectAsset(filename);
    if (activeRawData) {
      downloadExtractedFile(activeRawData, filename);
    }
    return;
  }

  const row = e.target.closest('tr');
  if (row && row.getAttribute('data-name')) {
    selectAsset(row.getAttribute('data-name'));
  }
});

// Extract Selected Button
btnExtractSelected.addEventListener('click', () => {
  if (!activeEntry) return;

  if (!activeRawData) {
    activeRawData = currentHandle.extract_file(activeEntry.name);
  }
  downloadExtractedFile(activeRawData, activeEntry.name);
});

// Hex vs Text Toggle
btnViewHex.addEventListener('click', () => {
  currentViewMode = 'hex';
  btnViewHex.classList.add('active');
  btnViewText.classList.remove('active');
  renderInspectorContent();
});

btnViewText.addEventListener('click', () => {
  currentViewMode = 'text';
  btnViewText.classList.add('active');
  btnViewHex.classList.remove('active');
  renderInspectorContent();
});

// Window Drag and Drop
window.addEventListener('dragover', (e) => e.preventDefault());
window.addEventListener('drop', async (e) => {
  e.preventDefault();
  const file = e.dataTransfer.files[0];
  if (file && file.name.endsWith('.tre')) {
    const buffer = await file.arrayBuffer();
    loadArchiveData(new Uint8Array(buffer), file.name);
  }
});

// Start initialization and auto-demonstrate demo archive
async function bootstrap() {
  await initWasm();
  try {
    const res = await fetch('assets/demo_archive.tre');
    if (res.ok) {
      const buffer = await res.arrayBuffer();
      loadArchiveData(new Uint8Array(buffer), 'demo_archive.tre');
      return;
    }
  } catch (err) {
    console.warn('Physical archive fetch bypassed:', err);
  }

  // In-memory fallback if assets directory is not hosted
  try {
    const demoBytes = create_demo_tre();
    loadArchiveData(demoBytes, 'demo_archive.tre');
  } catch (err) {
    console.error('Demo generation failed:', err);
  }
}

bootstrap();
