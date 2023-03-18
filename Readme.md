## mpegts-finder

## Usage

```bash
# 1. Generate hash file
mtf hash <full_video_file> -o <output_of_hash_file>

# 2. match block
mtf match <hash_file> <segment_to_match>

# 3. cut from full video file
mtf cut --from=<start> --to=<end> <full_video_file> <output_of_segment>
```
