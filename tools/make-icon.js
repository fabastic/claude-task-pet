// Draws the pixel-crab app icon and writes icon.ico (16, 24, 32, 48, 64, 128, 256 px PNGs inside).
// Usage: node tools/make-icon.js icon.ico
const fs = require('fs'), zlib = require('zlib')

// 16x16 pattern, same as the tray icon. o = body, k = eyes
const P = [
  '................', '................', '................', '...oooooooooo...',
  '...oooooooooo...', '...ookoooookoo..', '...ookoooookoo..', '.oooooooooooooo.',
  '.oooooooooooooo.', '...oooooooooo...', '...oooooooooo...', '...o.o....o.o...',
  '...o.o....o.o...', '................', '................', '................',
]
const COLORS = { o: [0xD9, 0x77, 0x57, 255], k: [0x1a, 0x1a, 0x1a, 255] }

function crc32(buf) {
  let c, crc = ~0
  for (const b of buf) { c = (crc ^ b) & 0xff; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; crc = (crc >>> 8) ^ c }
  return ~crc >>> 0
}
function chunk(type, data) {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length)
  const td = Buffer.concat([Buffer.from(type), data]); const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(td))
  return Buffer.concat([len, td, crc])
}
function png(size) {
  const rows = []
  for (let y = 0; y < size; y++) {
    const row = Buffer.alloc(1 + size * 4)
    for (let x = 0; x < size; x++) {
      const ch = P[Math.floor(y * 16 / size)][Math.floor(x * 16 / size)]
      Buffer.from(COLORS[ch] || [0, 0, 0, 0]).copy(row, 1 + x * 4)
    }
    rows.push(row)
  }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(size, 0); ihdr.writeUInt32BE(size, 4); ihdr[8] = 8; ihdr[9] = 6
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(Buffer.concat(rows))), chunk('IEND', Buffer.alloc(0))])
}

const sizes = [16, 24, 32, 48, 64, 128, 256], images = sizes.map(png)
const header = Buffer.alloc(6); header.writeUInt16LE(1, 2); header.writeUInt16LE(sizes.length, 4)
let offset = 6 + 16 * sizes.length
const dir = sizes.map((s, i) => {
  const e = Buffer.alloc(16); e[0] = s & 0xff; e[1] = s & 0xff; e.writeUInt16LE(1, 4); e.writeUInt16LE(32, 6)
  e.writeUInt32LE(images[i].length, 8); e.writeUInt32LE(offset, 12); offset += images[i].length; return e
})
fs.writeFileSync(process.argv[2] || 'icon.ico', Buffer.concat([header, ...dir, ...images]))
