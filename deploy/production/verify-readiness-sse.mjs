import { readFileSync } from 'node:fs';

const responsePath = process.argv[2];
if (!responsePath) {
  console.error('usage: node verify-readiness-sse.mjs <response-file>');
  process.exit(2);
}

const frames = readFileSync(responsePath, 'utf8').replace(/\r\n/g, '\n').split('\n\n');
let completed = false;

for (const frame of frames) {
  let event = 'message';
  const data = [];
  for (const line of frame.split('\n')) {
    if (line.startsWith('event:')) event = line.slice('event:'.length).trim();
    if (line.startsWith('data:')) data.push(line.slice('data:'.length).trimStart());
  }
  if (!data.length) continue;
  const payload = data.join('\n');
  if (event === 'error') {
    console.error('readiness SSE returned an error frame:', payload.slice(0, 300));
    process.exit(1);
  }
  if (event !== 'message') continue;
  let value;
  try {
    value = JSON.parse(payload);
  } catch {
    console.error('readiness SSE returned malformed message JSON:', payload.slice(0, 300));
    process.exit(1);
  }
  if (value?.type === 'done') completed = true;
}

if (!completed) {
  console.error('readiness SSE ended without a message { type: "done" } frame');
  process.exit(1);
}
