import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

// Run the shipping helper directly without building or starting the application.
const source = readFileSync(process.env.TAGREX_ARTWORK_ROTATION_SOURCE || new URL('../../app/ui/components/artwork.ts', import.meta.url), 'utf8');
const script = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText.replace(/^export /gm, '');
// Synthetic 7×5 RGBA grid, encoded with Pillow 13. JPEG additionally carries
// EXIF orientation 6 and an sRGB ICC profile. No collection artwork is used.
const encodedFixtures = {"gif": "R0lGODdhBwAFAIUAAAAAAAAzFQBmKgCZPwDMVCUAFSUzKiVmPyWZVCXMaUoAKkozP0pmVEqZaUrMfm8AP28zVG9maW+Zfm/Mk5QAVJQzaZRmfpSZk5TMqLkAabkzfrlmk7mZqLnMvd4Aft4zk95mqN6Zvd7M0gAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACwAAAAABwAFAAAIKgABFFDwgEIGDwEMLIBQQcMHAQcYRLCwAcQABA0kXOAQgkACBxMwdBAREAA7", "bmp": "Qk3CAAAAAAAAADYAAAAoAAAABwAAAAUAAAABACAAAAAAAIwAAADEDgAAxA4AAAAAAAAAAAAAVMwA/2nMJf9+zEqAk8xv/6jMlP+9zLmA0sze/z+ZAIBUmSX/aZlK/36Zb4CTmZT/qJm5/72Z3oAqZgD/P2YlgFRmSv9pZm//fmaUgJNmuf+oZt7/FTMA/yozJf8/M0qAVDNv/2kzlP9+M7mAkzPe/wAAAIAVACX/KgBK/z8Ab4BUAJT/aQC5/34A3oA=", "avif": "AAAAIGZ0eXBhdmlmAAAAAGF2aWZtaWYxbWlhZk1BMUIAAAGGbWV0YQAAAAAAAAAhaGRscgAAAAAAAAAAcGljdAAAAAAAAAAAAAAAAAAAAAAOcGl0bQAAAAAAAQAAACxpbG9jAAAAAEQAAAIAAQAAAAEAAAHiAAAAOwACAAAAAQAAAa4AAAA0AAAAQmlpbmYAAAAAAAIAAAAaaW5mZQIAAAAAAQAAYXYwMUNvbG9yAAAAABppbmZlAgAAAAACAABhdjAxQWxwaGEAAAAAGmlyZWYAAAAAAAAADmF1eGwAAgABAAEAAADDaXBycAAAAJ1pcGNvAAAAFGlzcGUAAAAAAAAABwAAAAUAAAAQcGl4aQAAAAADCAgIAAAADGF2MUOBAAwAAAAAE2NvbHJuY2x4AAEADQAGgAAAAA5waXhpAAAAAAEIAAAADGF2MUOBABwAAAAAOGF1eEMAAAAAdXJuOm1wZWc6bXBlZ0I6Y2ljcDpzeXN0ZW1zOmF1eGlsaWFyeTphbHBoYQAAAAAeaXBtYQAAAAAAAAACAAEEAQKDBAACBAEFhgcAAAB3bWRhdBIACgUYCLRhUDIpFAAMMQDtv0DJrFd5W614d84xxtNPqp1it6q9CdQ5mupqka0P0qsmat4SAAoIGAi0aICGg0IyLRTHh4ZlAgggnlAAAABIWujBC5IDISrPWH///b/pG1BDkHbAPqPZmIrcTyutYA==", "jpeg_oriented_icc": "/9j/4AAQSkZJRgABAQAAAQABAAD/4QAiRXhpZgAATU0AKgAAAAgAAQESAAMAAAABAAYAAAAAAAD/4gJcSUNDX1BST0ZJTEUAAQEAAAJMbGNtcwRAAABtbnRyUkdCIFhZWiAH6gALAA8ADwA7ABFhY3NwQVBQTAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA9tYAAQAAAADTLWxjbXMAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAtkZXNjAAABCAAAADZjcHJ0AAABQAAAAEx3dHB0AAABjAAAABRjaGFkAAABoAAAACxyWFlaAAABzAAAABRiWFlaAAAB4AAAABRnWFlaAAAB9AAAABRyVFJDAAACCAAAACBnVFJDAAACCAAAACBiVFJDAAACCAAAACBjaHJtAAACKAAAACRtbHVjAAAAAAAAAAEAAAAMZW5VUwAAABoAAAAcAHMAUgBHAEIAIABiAHUAaQBsAHQALQBpAG4AAG1sdWMAAAAAAAAAAQAAAAxlblVTAAAAMAAAABwATgBvACAAYwBvAHAAeQByAGkAZwBoAHQALAAgAHUAcwBlACAAZgByAGUAZQBsAHlYWVogAAAAAAAA9tYAAQAAAADTLXNmMzIAAAAAAAEMQgAABd7///MlAAAHkwAA/ZD///uh///9ogAAA9wAAMBuWFlaIAAAAAAAAG+gAAA49QAAA5BYWVogAAAAAAAAJJ8AAA+EAAC2w1hZWiAAAAAAAABilwAAt4cAABjZcGFyYQAAAAAAAwAAAAJmZgAA8qcAAA1ZAAAT0AAACltjaHJtAAAAAAADAAAAAKPXAABUewAATM0AAJmaAAAmZgAAD1z/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAAFAAcDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwCbwz4E0ryxx2/u0UUVNGrPkWo8nzHFfU4/vGf/2Q=="};

test.beforeEach(async ({ page }) => {
  await page.goto('about:blank');
  await page.addScriptTag({ content: `${script}\nwindow.artworkRotation = { rotateArtwork, decodeArtwork };` });
  await page.evaluate(() => {
    window.rotationFixture = (mime = 'image/png', width = 7, height = 5) => {
      const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
      const ctx = canvas.getContext('2d'); const pixels = ctx.createImageData(width, height);
      for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
        const at = (y * width + x) * 4;
        pixels.data.set([(x * 37) % 256, (y * 51) % 256, ((x + y) * 21) % 256, (x + y) % 3 === 0 ? 128 : 255], at);
      }
      ctx.putImageData(pixels, 0, 0);
      return { mime, data_base64: canvas.toDataURL(mime, .95).split(',')[1] };
    };
    window.rotationPixels = async (cover) => {
      const image = await window.artworkRotation.decodeArtwork(cover);
      const canvas = document.createElement('canvas'); canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
      const ctx = canvas.getContext('2d'); ctx.drawImage(image, 0, 0);
      return { width: canvas.width, height: canvas.height, pixels: [...ctx.getImageData(0, 0, canvas.width, canvas.height).data] };
    };
    // The original browser raster/encoder path provides a decoded-pixel parity
    // oracle for lossy input and profiles; it is never used by production code.
    window.rotationLegacy = async (cover, angle, image) => {
      const canvas = document.createElement('canvas'); const swapped = angle === 90 || angle === 270;
      canvas.width = swapped ? image.naturalHeight : image.naturalWidth;
      canvas.height = swapped ? image.naturalWidth : image.naturalHeight;
      const ctx = canvas.getContext('2d'); ctx.translate(canvas.width / 2, canvas.height / 2); ctx.rotate(angle * Math.PI / 180);
      ctx.drawImage(image, -image.naturalWidth / 2, -image.naturalHeight / 2);
      const mime = cover.mime === 'image/jpeg' ? 'image/jpeg' : 'image/png';
      return { ...cover, mime, data_base64: canvas.toDataURL(mime, .95).split(',')[1] };
    };
  });
});

test('zero and full turns preserve the exact cover object and bytes without decoding', async ({ page }) => {
  expect(await page.evaluate(async () => {
    const cover = { mime: 'image/unknown', data_base64: 'unchanged invalid image bytes', extra: true };
    return Promise.all([0, 360, -360, 720].map(async angle => (await window.artworkRotation.rotateArtwork(cover, angle)) === cover));
  })).toEqual([true, true, true, true]);
});

for (const degrees of [90, 180, 270, -90, 450]) {
  test(`rotates all nonsquare PNG edge pixels and alpha at ${degrees} degrees`, async ({ page }) => {
    const result = await page.evaluate(async degrees => {
      const cover = window.rotationFixture(); const before = await window.rotationPixels(cover);
      const output = await window.artworkRotation.rotateArtwork(cover, degrees);
      return { before, after: await window.rotationPixels(output), mime: output.mime };
    }, degrees);
    const { before, after } = result; const angle = (degrees % 360 + 360) % 360;
    expect(result.mime).toBe('image/png');
    expect([after.width, after.height]).toEqual(angle === 180 ? [7, 5] : [5, 7]);
    for (let y = 0; y < before.height; y++) for (let x = 0; x < before.width; x++) {
      const [dx, dy] = angle === 90 ? [before.height - 1 - y, x] : angle === 180 ? [before.width - 1 - x, before.height - 1 - y] : [y, before.width - 1 - x];
      const start = (y * before.width + x) * 4; const dest = (dy * after.width + dx) * 4;
      expect(after.pixels.slice(dest, dest + 4)).toEqual(before.pixels.slice(start, start + 4));
    }
  });
}

test('browser codecs, JPEG quality, EXIF orientation, ICC and PNG fallback match the old decoded pixels', async ({ page }) => {
  const results = await page.evaluate(async encoded => {
    const fixtures = ['image/png', 'image/jpeg', 'image/webp'].map(mime => window.rotationFixture(mime));
    for (const [format, data_base64] of Object.entries(encoded)) fixtures.push({ mime: format === 'jpeg_oriented_icc' ? 'image/jpeg' : `image/${format}`, data_base64 });
    const results = [];
    for (const cover of fixtures) {
      const image = await window.artworkRotation.decodeArtwork(cover);
      for (const angle of [90, 180, 270]) {
        const legacy = await window.rotationLegacy(cover, angle, image);
        // This invalid replacement source ensures the cached image is used.
        const output = await window.artworkRotation.rotateArtwork({ ...cover, data_base64: 'cached-only' }, angle, image);
        results.push({ input: cover.mime, mime: output.mime, actual: await window.rotationPixels(output), expected: await window.rotationPixels(legacy) });
      }
    }
    return results;
  }, encodedFixtures);
  expect(results).toHaveLength(21);
  for (const result of results) {
    expect(result.mime).toBe(result.input === 'image/jpeg' ? 'image/jpeg' : 'image/png');
    expect(result.actual).toEqual(result.expected);
  }
});

test('encoding failures release the canvas and a subsequent attempt can succeed', async ({ page }) => {
  const results = await page.evaluate(async () => {
    const { rotateArtwork, decodeArtwork } = window.artworkRotation;
    const cover = window.rotationFixture(); const image = await decodeArtwork(cover);
    const originalCreate = document.createElement.bind(document); const NativeReader = window.FileReader;
    const results = [];
    for (const failure of ['context', 'null-blob', 'throw-blob', 'reader-error', 'reader-abort']) {
      let canvas;
      document.createElement = (...args) => {
        const element = originalCreate(...args);
        if (args[0] === 'canvas') {
          canvas = element;
          if (failure === 'context') element.getContext = () => null;
          if (failure === 'null-blob') element.toBlob = callback => setTimeout(() => callback(null));
          if (failure === 'throw-blob') element.toBlob = () => { throw new Error('encoder threw'); };
        }
        return element;
      };
      if (failure.startsWith('reader-')) window.FileReader = class {
        readAsDataURL() { setTimeout(() => failure === 'reader-error' ? this.onerror() : this.onabort()); }
      };
      try { await rotateArtwork(cover, 90, image); results.push({ failure, rejected: false }); }
      catch (error) { results.push({ failure, rejected: !!error.message, released: canvas.width === 0 && canvas.height === 0 }); }
      finally { document.createElement = originalCreate; window.FileReader = NativeReader; }
      const retry = await window.rotationPixels(await rotateArtwork(cover, 90, image));
      results.at(-1).retry = [retry.width, retry.height];
    }
    let decodeRejected = false;
    try { await rotateArtwork({ mime: 'image/png', data_base64: 'invalid' }, 90); } catch { decodeRejected = true; }
    return { results, decodeRejected };
  });
  expect(results.decodeRejected).toBe(true);
  for (const result of results.results) expect(result).toMatchObject({ rejected: true, released: true, retry: [5, 7] });
});

test('final encoding yields to the event loop instead of calling the synchronous data URL encoder', async ({ page }) => {
  expect(await page.evaluate(async () => {
    const cover = window.rotationFixture(); const image = await window.artworkRotation.decodeArtwork(cover);
    HTMLCanvasElement.prototype.toDataURL = () => { throw new Error('synchronous encoder called'); };
    let ticked = false;
    const pending = window.artworkRotation.rotateArtwork(cover, 90, image);
    setTimeout(() => { ticked = true; });
    await pending;
    return ticked;
  })).toBe(true);
});
