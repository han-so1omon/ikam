export const describeMime = (mimeType: string | null | undefined): string => {
  const mime = (mimeType ?? '').toLowerCase();
  if (!mime) return 'unknown';
  if (mime === 'text/markdown') return 'markdown';
  if (mime === 'application/json') return 'json';
  if (mime === 'application/pdf') return 'pdf file';
  if (mime.startsWith('image/')) return `${mime.split('/')[1] ?? 'image'} image`;
  if (mime === 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet') return 'spreadsheet';
  if (mime === 'application/vnd.openxmlformats-officedocument.presentationml.presentation') return 'slide deck';
  if (mime === 'application/vnd.openxmlformats-officedocument.wordprocessingml.document') return 'word document';
  if (mime === 'application/ikam-table-region+json') return 'table section';
  if (mime === 'application/ikam-slide-shape+json') return 'slide section';
  if (mime === 'application/ikam-pdf-page+json') return 'pdf page';
  if (mime === 'text/ikam-paragraph') return 'paragraph';
  return mime;
};
