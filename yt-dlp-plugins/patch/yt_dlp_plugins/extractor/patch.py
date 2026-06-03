import json
import re as _re

from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import (
    ExtractorError,
    UserNotLive,
    lowercase_escape,
    traverse_obj,
)


class PatchedIE(InfoExtractor):
    _VALID_URL = r'https?://(?:[a-z]{2}\.)?' + ''.join(chr(c) for c in [115, 116, 114, 105, 112, 99, 104, 97, 116]) + r'\.com/(?P<id>[^/?#]+)'
    _TESTS = []
    _API_HEADERS = {
        'Content-Type': 'application/x-www-form-urlencoded',
        'X-Requested-With': 'XMLHttpRequest',
    }

    def _real_extract(self, url):
        video_id = self._match_id(url)

        webpage = self._download_webpage(url, video_id, headers=self.geo_verification_headers())

        data = self._search_json(
            r'<script\b[^>]*>\s*window\.__PRELOADED_STATE__\s*=',
            webpage, 'data', video_id, transform_source=lowercase_escape)

        if not traverse_obj(data, ('viewCam', 'model', 'isLive', {bool})):
            raise UserNotLive(video_id=video_id)

        # Try API first to get the real streamName
        api_formats = self._try_api(video_id, url)
        if api_formats:
            return {
                'id': video_id,
                'title': video_id,
                'is_live': True,
                'formats': api_formats,
                'age_limit': 18,
            }

        # Fallback to page data
        stream_name = traverse_obj(data, ('viewCam', 'streamName', {str})) or str(data['viewCam']['model']['id'])
        self.to_screen(f'[Patched] Page streamName={stream_name!r}')

        # Look for the real stream URL in the page JS
        all_streams = _re.findall(r'(?:https?://[^"\']*(?:m3u8|m4s|ts)[^"\']*)', webpage)
        self.to_screen(f'[Patched] m3u8 URLs in page: {all_streams}')

        formats = self._try_all_cdns(stream_name, video_id)
        if formats:
            return {
                'id': video_id,
                'title': video_id,
                'is_live': True,
                'formats': formats,
                'age_limit': 18,
            }

        self.raise_no_formats('Unable to extract stream', video_id=video_id)

    def _try_api(self, username, page_url):
        api_url = f'https://stripchat.com/api/front/v2/models/username/{username}/cam'
        headers = {**self._API_HEADERS, 'Referer': page_url}
        try:
            resp = self._download_webpage(api_url, username, headers=headers, fatal=False)
            if not resp:
                return None
            api_data = json.loads(resp)
            cam = api_data.get('cam') or {}
            stream_name = cam.get('streamName')
            user_info = api_data.get('user', {}).get('user', {})
            is_live = user_info.get('isLive', False)
            status = user_info.get('status', '')

            self.to_screen(f'[Patched] API: streamName={stream_name!r} live={is_live} status={status}')

            if not is_live or status != 'public':
                return None

            if not stream_name:
                return None

            return self._try_all_cdns(stream_name, username)
        except Exception as e:
            self.to_screen(f'[Patched] API error: {e}')
            return None

    def _try_all_cdns(self, stream_name, video_id):
        prefixes = ['edge-hls', 'media-hls', 'b-hls-24', 'b-hls-23', 'b-hls-25']
        cdns = ['doppiocdn.com', 'doppiocdn1.com', 'doppiocdn.media', 'doppiocdn.net', 'doppiocdn.org', 'doppiocdn.live']
        suffixes = ['_auto', '']
        paths = [
            f'hls/{stream_name}/master/{stream_name}{{suffix}}.m3u8',
            f'hls/{stream_name}/{stream_name}.m3u8',
            f'b-hls-24/{stream_name}/{stream_name}.m3u8',
        ]
        query_params = ['', '?playlistType=standard', '?playlistType=lowLatency']

        for cdn in cdns:
            for prefix in prefixes:
                for spath in paths:
                    for suffix in suffixes:
                        for qp in query_params:
                            url = f'https://{prefix}.{cdn}/{spath.format(suffix=suffix)}{qp}'
                            found = self._extract_m3u8_formats(
                                url, video_id, ext='mp4', m3u8_id='hls', fatal=False, live=True)
                            if found:
                                self.to_screen(f'[Patched] FOUND {len(found)} from {prefix}.{cdn}')
                                return found
        return None
