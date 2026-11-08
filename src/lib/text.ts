/** How far into a file the binary sniff looks. */
const SNIFF_BYTES = 8192;

/** Decode file bytes for display or comparison, or null when they look
 *  binary. A UTF-16 BOM marks text even though its ASCII is full of NULs
 *  (decoded per the BOM, which is dropped); otherwise a NUL byte in the
 *  first 8 KB means binary, and the rest decodes as UTF-8. */
export function decodeText(buf: ArrayBuffer): string | null {
  const bytes = new Uint8Array(buf);
  const utf16 =
    bytes[0] === 0xff && bytes[1] === 0xfe
      ? "utf-16le"
      : bytes[0] === 0xfe && bytes[1] === 0xff
        ? "utf-16be"
        : null;
  if (utf16) return new TextDecoder(utf16).decode(bytes);
  if (bytes.subarray(0, SNIFF_BYTES).includes(0)) return null;
  return new TextDecoder().decode(bytes);
}
