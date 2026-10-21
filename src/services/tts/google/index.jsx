import { fetch, ResponseType } from '@tauri-apps/api/http';

// Google Translate TTS endpoint accepts at most 200 characters per request
const MAX_LENGTH = 200;

export async function tts(text, lang, options = {}) {
    const res = await fetch('https://translate.google.com/translate_tts', {
        method: 'GET',
        query: {
            ie: 'UTF-8',
            client: 'tw-ob',
            tl: lang,
            q: text.slice(0, MAX_LENGTH),
        },
        responseType: ResponseType.Binary,
    });

    if (res.ok) {
        return res.data;
    } else {
        throw `Http Request Error\nHttp Status: ${res.status}`;
    }
}

export * from './Config';
export * from './info';
