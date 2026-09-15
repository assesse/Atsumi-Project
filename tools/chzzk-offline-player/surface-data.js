// Explicitly synthetic. No channel, account or saved user chat is accessed.
export const fixture = {
  title: '로컬 영상으로 확인하는 CHZZK 원본 방송 화면',
  channelName: '오프라인 테스트 채널',
  profileImage: '/assets/default_profile_dark.png',
  viewers: 1234,
  uptime: '01:23:45',
  messages: Array.from({ length: 60 }, (_, index) => {
    const names = ['초록빛', '달빛산책', '보라구름', '귤한조각', '테스트 채널', '느긋한오후'];
    const colors = ['#66d1a8', '#9bbcff', '#cf91de', '#edb95c', '#00ffa3', '#b4caa0'];
    const texts = ['안녕하세요! 원본 채팅 UI 확인 중입니다.', '영상 옆 채팅창도 로컬 파일로 표시돼요.', '닉네임 색상과 줄 간격을 확인해 보세요.', 'ㅋㅋㅋㅋㅋㅋ', '조금 긴 채팅은 이렇게 여러 줄로 이어져도 원본과 같은 여백과 글자 크기를 유지하는지 확인할 수 있어요.', '메뉴와 배지에 마우스를 올려보세요.'];
    const user = index % names.length;
    return {
      key: `offline-${index}`, user: `fixture-${user}`, time: 100000 + index * 1000, type: 1,
      status: 'NORMAL', content: texts[user], extras: '{}',
      profile: { nickname: names[user], userIdHash: `fixture-${user}`, userRoleCode: user === 4 ? 'streamer' : 'common_user',
        ...(user === 4 ? { badge: { imageUrl: '/assets/icon_official_mark.png' }, title: { name: '원본 아이콘 표시 테스트' } } : {}) },
      displayBadgeList: [], displayNicknameColor: { dark: colors[user], light: colors[user] },
    };
  }),
};
