// Replay-only behavior. The original live chat and player remain independent.
const normalize = value => String(value ?? '').normalize('NFKC').toLocaleLowerCase('ko-KR').replace(/\s+/gu, ' ').trim();
export const searchFields = Object.freeze([['all', '전체'], ['body', '본문'], ['nickname', '닉네임']]);

export function createChatSearchIndex(messages) {
  return messages.map(message => ({
    message,
    body: normalize(message.content),
    nickname: normalize(message.profile?.nickname),
  }));
}

export function searchChatMessages(index, query, field = 'all') {
  const needle = normalize(query);
  const selected = searchFields.some(([value]) => value === field) ? field : 'all';
  return index.filter(entry => !needle ||
    (selected !== 'nickname' && entry.body.includes(needle)) ||
    (selected !== 'body' && entry.nickname.includes(needle))).map(entry => entry.message);
}

export function createReplaySearch(K, styles) {
  return function ReplaySearch({ query, count, countText, onQuery }) {
    const inputRef = K.useRef(null);
    const clear = () => { onQuery(''); inputRef.current?.focus(); };
    return K.createElement('section', { className: 'replay-search', 'aria-label': '저장된 채팅 검색' },
      normalize(query) && K.createElement('span', { className: 'replay-search__announcement', role: 'status', 'aria-live': 'polite', 'aria-atomic': 'true' },
        countText ?? `${count.toLocaleString('ko-KR')}개 결과`),
      K.createElement('div', { className: `${styles.container} replay-search__input` },
        K.createElement('input', {
          ref: inputRef, type: 'search', className: styles.input, value: query,
          'aria-label': '검색', placeholder: '검색',
          autoComplete: 'off', spellCheck: false, maxLength: 256,
          onChange: event => onQuery(event.target.value),
          onKeyDown: event => {
            if (event.nativeEvent.isComposing) return;
            if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); clear(); }
            if (event.key === 'Enter') event.preventDefault();
          },
        }),
        query && K.createElement('button', { type: 'button', className: 'replay-search__clear', onClick: clear, 'aria-label': '검색 지우기', title: '검색 지우기 (Esc)' }, '지우기')));
  };
}
