// Minimal PNG encoder/decoder for the dynamic image tests (node:zlib only).
// Encodes a solid-color RGBA image for the fake upstream to serve, and
// decodes the 8-bit RGBA / RGB, non-interlaced PNGs the renderer writes, so a
// test can check the size and single pixels of an image it downloads.

import { deflateSync, inflateSync } from "node:zlib"

const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])

const CRC_TABLE = (() => {
  const table = new Uint32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    }
    table[n] = c >>> 0
  }
  return table
})()

const crc32 = (data: Buffer): number => {
  let crc = 0xffffffff
  for (const byte of data) {
    crc = (CRC_TABLE[(crc ^ byte) & 0xff] ?? 0) ^ (crc >>> 8)
  }
  return (crc ^ 0xffffffff) >>> 0
}

const chunk = (type: string, data: Buffer): Buffer => {
  const length = Buffer.alloc(4)
  length.writeUInt32BE(data.length)
  const body = Buffer.concat([Buffer.from(type, "ascii"), data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(body))
  return Buffer.concat([length, body, crc])
}

export type Rgba = [number, number, number, number]

/** A `width` x `height` PNG filled with one RGBA color. */
export const solidPng = (width: number, height: number, color: Rgba): Buffer => {
  const header = Buffer.alloc(13)
  header.writeUInt32BE(width, 0)
  header.writeUInt32BE(height, 4)
  header[8] = 8 // bit depth
  header[9] = 6 // RGBA
  header[10] = 0
  header[11] = 0
  header[12] = 0
  const row = Buffer.alloc(1 + width * 4)
  for (let x = 0; x < width; x++) {
    row.set(color, 1 + x * 4)
  }
  const raw = Buffer.concat(Array.from({ length: height }, () => row))
  return Buffer.concat([
    SIGNATURE,
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ])
}

export type DecodedPng = {
  width: number
  height: number
  /** RGBA of pixel (x, y); RGB images report alpha 255. */
  pixel: (x: number, y: number) => Rgba
}

const paeth = (a: number, b: number, c: number): number => {
  const p = a + b - c
  const pa = Math.abs(p - a)
  const pb = Math.abs(p - b)
  const pc = Math.abs(p - c)
  if (pa <= pb && pa <= pc) {
    return a
  }
  return pb <= pc ? b : c
}

/** Decodes an 8-bit RGBA or RGB, non-interlaced PNG. Throws on anything else. */
export const decodePng = (input: Uint8Array): DecodedPng => {
  const bytes = Buffer.from(input)
  if (!bytes.subarray(0, 8).equals(SIGNATURE)) {
    throw new Error("not a PNG")
  }
  let offset = 8
  let width = 0
  let height = 0
  let colorType = 0
  const idat: Buffer[] = []
  while (offset < bytes.length) {
    const length = bytes.readUInt32BE(offset)
    const type = bytes.toString("ascii", offset + 4, offset + 8)
    const data = bytes.subarray(offset + 8, offset + 8 + length)
    if (type === "IHDR") {
      width = data.readUInt32BE(0)
      height = data.readUInt32BE(4)
      const bitDepth = data[8]
      colorType = data[9] ?? 0
      if (bitDepth !== 8 || (colorType !== 6 && colorType !== 2) || data[12] !== 0) {
        throw new Error(`unsupported PNG: bit depth ${bitDepth}, color type ${colorType}, interlace ${data[12]}`)
      }
    } else if (type === "IDAT") {
      idat.push(data)
    } else if (type === "IEND") {
      break
    }
    offset += 12 + length
  }
  const channels = colorType === 6 ? 4 : 3
  const stride = width * channels
  const raw = inflateSync(Buffer.concat(idat))
  const pixels = Buffer.alloc(stride * height)
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)]
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1))
    for (let x = 0; x < stride; x++) {
      const left = x >= channels ? (pixels[y * stride + x - channels] ?? 0) : 0
      const up = y > 0 ? (pixels[(y - 1) * stride + x] ?? 0) : 0
      const upLeft = y > 0 && x >= channels ? (pixels[(y - 1) * stride + x - channels] ?? 0) : 0
      const value = line[x] ?? 0
      let out: number
      switch (filter) {
        case 0:
          out = value
          break
        case 1:
          out = value + left
          break
        case 2:
          out = value + up
          break
        case 3:
          out = value + Math.floor((left + up) / 2)
          break
        case 4:
          out = value + paeth(left, up, upLeft)
          break
        default:
          throw new Error(`bad PNG filter ${filter}`)
      }
      pixels[y * stride + x] = out & 0xff
    }
  }
  return {
    width,
    height,
    pixel: (x, y) => {
      const start = y * stride + x * channels
      return [
        pixels[start] ?? 0,
        pixels[start + 1] ?? 0,
        pixels[start + 2] ?? 0,
        channels === 4 ? (pixels[start + 3] ?? 0) : 255,
      ]
    },
  }
}

/** Downloads a URL and returns status, media type and bytes. */
export const download = async (url: string): Promise<{ status: number; contentType: string; bytes: Buffer }> => {
  const response = await fetch(url, { redirect: "manual" })
  const bytes = Buffer.from(await response.arrayBuffer())
  return {
    status: response.status,
    contentType: (response.headers.get("content-type") ?? "").split(";")[0]?.trim().toLowerCase() ?? "",
    bytes,
  }
}
