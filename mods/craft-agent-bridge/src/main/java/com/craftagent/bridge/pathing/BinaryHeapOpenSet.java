package com.craftagent.bridge.pathing;

import java.util.Arrays;

public class BinaryHeapOpenSet {
    private PathNode[] heap;
    private int size;

    public BinaryHeapOpenSet(int capacity) {
        this.heap = new PathNode[capacity];
        this.size = 0;
    }

    public void push(PathNode node) {
        if (size >= heap.length) heap = Arrays.copyOf(heap, heap.length * 2);
        heap[size] = node;
        int i = size++;
        while (i > 0) {
            int p = (i - 1) / 2;
            if (heap[i].compareTo(heap[p]) >= 0) break;
            PathNode tmp = heap[i]; heap[i] = heap[p]; heap[p] = tmp;
            i = p;
        }
    }

    public PathNode pop() {
        if (size == 0) return null;
        PathNode result = heap[0];
        heap[0] = heap[--size];
        int i = 0;
        while (true) {
            int smallest = i;
            int left = 2 * i + 1;
            int right = 2 * i + 2;
            if (left < size && heap[left].compareTo(heap[smallest]) < 0) smallest = left;
            if (right < size && heap[right].compareTo(heap[smallest]) < 0) smallest = right;
            if (smallest == i) break;
            PathNode tmp = heap[i]; heap[i] = heap[smallest]; heap[smallest] = tmp;
            i = smallest;
        }
        return result;
    }

    public boolean isEmpty() { return size == 0; }
    public int size() { return size; }
}
