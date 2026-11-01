use anahata_audio::ring::AudioRingBuffer;

#[test]
fn test_ring_buffer_push_pop_integrity() {
    let capacity = 1024;
    let (mut producer, mut consumer) = AudioRingBuffer::new(capacity);

    assert_eq!(producer.slots(), capacity);
    assert_eq!(consumer.slots(), 0);

    let input_samples: Vec<f32> = (0..500).map(|i| (i as f32) / 500.0).collect();

    // Push slice
    let mut chunk = producer.write_chunk(input_samples.len()).expect("write_chunk failed");
    let (first, second) = chunk.as_mut_slices();
    let n1 = first.len();
    first.copy_from_slice(&input_samples[..n1]);
    if !second.is_empty() {
        second.copy_from_slice(&input_samples[n1..]);
    }
    chunk.commit_all();

    assert_eq!(consumer.slots(), 500);

    // Read back
    let chunk = consumer.read_chunk(500).expect("read_chunk failed");
    let (first, second) = chunk.as_slices();
    let mut output_samples = Vec::new();
    output_samples.extend_from_slice(first);
    output_samples.extend_from_slice(second);
    chunk.commit_all();

    assert_eq!(output_samples, input_samples);
    assert_eq!(consumer.slots(), 0);
}

#[test]
fn test_ring_buffer_wrap_around() {
    let capacity = 128;
    let (mut producer, mut consumer) = AudioRingBuffer::new(capacity);

    for cycle in 0..10 {
        let chunk_data: Vec<f32> = (0..64).map(|i| (cycle * 64 + i) as f32).collect();

        let mut out = producer.write_chunk(64).unwrap();
        let (f, s) = out.as_mut_slices();
        let n = f.len();
        f.copy_from_slice(&chunk_data[..n]);
        if !s.is_empty() {
            s.copy_from_slice(&chunk_data[n..]);
        }
        out.commit_all();

        let in_chunk = consumer.read_chunk(64).unwrap();
        let (f, s) = in_chunk.as_slices();
        let mut read_data = Vec::new();
        read_data.extend_from_slice(f);
        read_data.extend_from_slice(s);
        in_chunk.commit_all();

        assert_eq!(read_data, chunk_data);
    }
}
