/** Keep the file's encoding marker and line endings across browser textarea normalization. */
export type TextFormat = {
  bom: boolean;
  ending: "LF" | "CRLF";
  mixed: boolean;
};
export function openText(text: string): { value: string; format: TextFormat } {
  const bom = text.startsWith("\uFEFF");
  const body = bom ? text.slice(1) : text;
  const crlf = body.includes("\r\n");
  return {
    value: body.replace(/\r\n/g, "\n"),
    format: {
      bom,
      ending: crlf ? "CRLF" : "LF",
      mixed: /\r(?!\n)/.test(body) || (crlf && /(^|[^\r])\n/.test(body)),
    },
  };
}
export function saveText(value: string, format: TextFormat): string {
  if (format.mixed) throw new Error("混合换行文本只支持预览");
  return (
    (format.bom ? "\uFEFF" : "") +
    (format.ending === "CRLF" ? value.replace(/\n/g, "\r\n") : value)
  );
}
