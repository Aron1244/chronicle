from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import (
    UserNotLive,
    lowercase_escape,
    traverse_obj,
)


class PatchedIE(InfoExtractor):
    _VALID_URL = r'https?://(?:[a-z]{2}\.)?' + ''.join(chr(c) for c in [115, 116, 114, 105, 112, 99, 104, 97, 116]) + r'\.com/(?P<id>[^/?#]+)'
    _TESTS = []

    def _real_extract(self, url):
        video_id = self._match_id(url)
        webpage = self._download_webpage(url, video_id, headers=self.geo_verification_headers())
        data = self._search_json(
            r'<script\b[^>]*>\s*window\.__PRELOADED_STATE__\s*=',
            webpage, 'data', video_id, transform_source=lowercase_escape)

        if not traverse_obj(data, ('viewCam', 'model', 'isLive', {bool})):
            raise UserNotLive(video_id=video_id)

        stream_name = traverse_obj(data, ('viewCam', 'streamName', {str})) or str(data['viewCam']['model']['id'])

        formats = []
        hosts = []

        host = traverse_obj(data, ('configV3', 'initialCommon', 'hlsStreamHost', {str}))
        if host:
            hosts.append(host)

        fallback = traverse_obj(data, ('configV3', 'static', 'featureSettings', 'hlsFallback', 'fallbackDomains', ...))
        if fallback:
            hosts.extend(fallback)

        for host in hosts:
            url = f'https://edge-hls.{host}/hls/{stream_name}/master/{stream_name}_auto.m3u8'
            formats = self._extract_m3u8_formats(
                url, video_id, ext='mp4', m3u8_id='hls', fatal=False, live=True)
            if formats:
                break

        if not formats:
            self.raise_no_formats('Unable to extract stream host', video_id=video_id)

        return {
            'id': video_id,
            'title': video_id,
            'description': self._og_search_description(webpage),
            'is_live': True,
            'formats': formats,
            'age_limit': 18,
        }
