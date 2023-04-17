#!/usr/bin/env python3
# Inline SVGs into README.md for crates.io release

import base64
import re
import urllib.parse

in_file = 'README.md'
out_file = '.cargo.README.md'

img_re = re.compile(r'<img(?:\s*)src="([^"]+)"(?:\s*)/>')

def svg_to_data_url(contents: bytes) -> str:
    # b64 = base64.b64encode(contents).decode('ascii')
    mediatype = 'image/svg+xml'
    # return f'data:{mediatype};base64,{b64}'
    contents = urllib.parse.quote_from_bytes(contents)
    return f'data:{mediatype},{contents}'


def main():
    with open(in_file) as f, open(out_file, 'w') as fw:
        for line in f:
            if (m := img_re.match(line)) is not None:
                svg_filename = m.group(1)
                print(svg_filename)
                with open(svg_filename, 'rb') as svg:
                    svg_contents = svg.read()
                data_url = svg_to_data_url(svg_contents)
                line = f"<img src='{data_url}'/>\n"
            fw.write(line)

if __name__ == '__main__':
    main()