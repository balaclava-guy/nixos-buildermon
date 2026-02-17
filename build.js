#!/usr/bin/env node
import { readFileSync, writeFileSync, copyFileSync, mkdirSync } from 'fs';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));

// Ensure dist directory exists
mkdirSync(join(__dirname, 'dist', 'assets'), { recursive: true });

// Copy xterm.js and CSS
const xtermPath = join(__dirname, 'node_modules', '@xterm', 'xterm');
copyFileSync(
  join(xtermPath, 'lib', 'xterm.js'),
  join(__dirname, 'dist', 'assets', 'xterm.js')
);
copyFileSync(
  join(xtermPath, 'css', 'xterm.css'),
  join(__dirname, 'dist', 'assets', 'xterm.css')
);

// Read and process index.html
const indexPath = join(__dirname, 'src', 'index.html');
const indexContent = readFileSync(indexPath, 'utf-8');

// Replace CDN links with local paths
const processedIndex = indexContent
  .replace(
    /https:\/\/cdn\.jsdelivr\.net\/npm\/xterm@[\d.]+\/lib\/xterm\.min\.js/g,
    '/assets/xterm.js'
  )
  .replace(
    /https:\/\/cdn\.jsdelivr\.net\/npm\/xterm@[\d.]+\/css\/xterm\.min\.css/g,
    '/assets/xterm.css'
  )
  .replace(
    /href="\/logo\.png"/g,
    'href="/assets/logo.png"'
  )
  .replace(
    /src="\/logo\.png"/g,
    'src="/assets/logo.png"'
  );

writeFileSync(join(__dirname, 'dist', 'index.html'), processedIndex);

// Copy logo
copyFileSync(
  join(__dirname, 'public', 'logo.png'),
  join(__dirname, 'dist', 'assets', 'logo.png')
);

console.log('✓ Build complete: dist/');
