import { invoke } from '@tauri-apps/api/tauri';
import React from 'react';

export default function SelectionIcon() {
    const translate = () => {
        invoke('selection_icon_translate');
    };

    return (
        <div
            className='h-screen w-screen flex items-center justify-center bg-transparent cursor-pointer select-none'
            onMouseEnter={translate}
            onClick={translate}
        >
            <img
                src='/icon.png'
                className='h-full w-full'
                draggable={false}
            />
        </div>
    );
}
