import urllib.request
import json
import os
import re

out_dir = os.path.join(os.getcwd(), 'docs', 'onchaindivers_archive')
os.makedirs(out_dir, exist_ok=True)

url = 'https://onchaindivers.substack.com/api/v1/posts?limit=50'
req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
with urllib.request.urlopen(req) as resp:
    data = json.loads(resp.read().decode('utf-8'))

posts = data.get('posts', [])
print(f'Downloading {len(posts)} articles...')

index_md = ['# Onchain Divers Archive - Complete Backup\n\n']
index_md.append(f'Total Articles: {len(posts)}\n\n')

for i, p in enumerate(posts, 1):
    title = p.get('title', 'untitled').strip()
    slug = p.get('slug', f'post_{i}')
    post_date = str(p.get('post_date', ''))[:10]
    canonical_url = p.get('canonical_url') or f'https://onchaindivers.substack.com/p/{slug}'
    subtitle = p.get('subtitle') or ''
    description = p.get('description') or ''
    body_html = p.get('body_html') or ''
    
    clean_title = re.sub(r'[^a-zA-Z0-9_\- ]', '', title).replace(' ', '_')
    filename = f'{i:02d}_{clean_title[:45]}.md'
    filepath = os.path.join(out_dir, filename)
    
    with open(filepath, 'w', encoding='utf-8') as f:
        f.write(f'# {title}\n\n')
        f.write(f'- **Date**: {post_date}\n')
        f.write(f'- **Original URL**: [{canonical_url}]({canonical_url})\n')
        if subtitle:
            f.write(f'- **Subtitle**: {subtitle}\n')
        if description:
            f.write(f'- **Description**: {description}\n')
        f.write('\n---\n\n')
        f.write(body_html)
    
    index_md.append(f'{i}. [{title}](./{filename}) - *{post_date}*\n')

with open(os.path.join(out_dir, 'README.md'), 'w', encoding='utf-8') as f:
    f.writelines(index_md)

print('Successfully archived all articles to docs/onchaindivers_archive!')
