import { downloadArtifact } from '../api/client';

const mockFetch = vi.fn();
let anchorClickSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  mockFetch.mockReset();
  vi.stubGlobal('fetch', mockFetch);
  vi.stubGlobal('URL', {
    ...URL,
    createObjectURL: vi.fn().mockReturnValue('blob:mock-url'),
    revokeObjectURL: vi.fn(),
  });
  anchorClickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
});

afterEach(() => {
  anchorClickSpy.mockRestore();
  vi.unstubAllGlobals();
});

test('downloadArtifact fetches blob and triggers browser download', async () => {
  const mockBlob = new Blob(['artifact-bytes'], { type: 'application/octet-stream' });
  mockFetch.mockResolvedValueOnce({
    ok: true,
    blob: async () => mockBlob,
    headers: new Headers({ 'content-disposition': 'attachment; filename="Revenue Plan.xlsx"' }),
  } as Response);

  const appendChildSpy = vi.spyOn(document.body, 'appendChild');
  const removeChildSpy = vi.spyOn(document.body, 'removeChild');
  await downloadArtifact('art-123');

  expect(mockFetch).toHaveBeenCalledWith(
    expect.stringContaining('/artifacts/art-123/download'),
    { method: 'GET' }
  );
  expect(URL.createObjectURL).toHaveBeenCalledWith(mockBlob);
  expect(appendChildSpy).toHaveBeenCalled();
  const anchor = appendChildSpy.mock.calls[0][0] as HTMLAnchorElement;
  expect(anchor.tagName).toBe('A');
  expect(anchor.download).toBe('Revenue Plan.xlsx');
  expect(anchor.href).toContain('blob:mock-url');
  expect(anchorClickSpy).toHaveBeenCalledTimes(1);
  expect(removeChildSpy).toHaveBeenCalledWith(anchor);
  expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:mock-url');
});

test('downloadArtifact uses fallback filename when content-disposition missing', async () => {
  const mockBlob = new Blob(['data'], { type: 'application/octet-stream' });
  mockFetch.mockResolvedValueOnce({
    ok: true,
    blob: async () => mockBlob,
    headers: new Headers(),
  } as Response);

  const appendChildSpy = vi.spyOn(document.body, 'appendChild');

  await downloadArtifact('art-456');

  const anchor = appendChildSpy.mock.calls[0][0] as HTMLAnchorElement;
  expect(anchor.download).toBe('artifact-art-456');
});

test('downloadArtifact throws on non-ok response', async () => {
  mockFetch.mockResolvedValueOnce({
    ok: false,
    status: 404,
  } as Response);

  await expect(downloadArtifact('art-missing')).rejects.toThrow('Download failed (404)');
});
