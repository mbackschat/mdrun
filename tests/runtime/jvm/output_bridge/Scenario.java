package io.github.mbackschat.mdrun.bridgefixture;

import java.io.IOException;
import java.nio.CharBuffer;

public final class Scenario {
    public static int conversions;
    public static final String CONCURRENT = "c".repeat(65536) + "\uD83D\uDE00" + "d".repeat(65536);

    private Scenario() {}

    public static int run(String name, Appendable out, Appendable err) throws IOException {
        return switch (name) {
            case "operations" -> {
                out.append("ASCII|").append('\uD83D').append('\uDE00').append("|\u00E4\u20AC|");
                err.append("err|\u00E9|").append('\uD83D').append('\uDE00');
                out.append((CharSequence) null).append((CharSequence) null, 1, 3);
                out.append("prefix-range-suffix", 7, 12);
                StringBuilder mutable = new StringBuilder("first");
                out.append(mutable);
                mutable.replace(0, mutable.length(), "second");
                out.append(mutable);
                CharBuffer selected = CharBuffer.wrap("skip\u20ACtail");
                selected.position(4).limit(5);
                out.append(selected);
                for (int[] range : new int[][] {{-1, 1}, {2, 1}, {0, 4}}) {
                    try {
                        out.append("abc", range[0], range[1]);
                        throw new AssertionError("accepted an invalid subsequence");
                    } catch (IndexOutOfBoundsException expected) { }
                }
                out.append("").append("empty", 2, 2);
                yield 0;
            }
            case "orphan" -> {
                out.append("tail").append('\uD83D');
                err.append('\uD83D');
                yield 0;
            }
            case "low" -> {
                out.append('\uDE00');
                yield 0;
            }
            case "status17" -> {
                err.append("ordinary failure\n");
                yield 17;
            }
            case "large" -> {
                conversions = 0;
                out.append(new CharSequence() {
                    private final String pattern = "X\uD83D\uDE00YZ";
                    @Override public int length() { return 1024 * 1024 + 17; }
                    @Override public char charAt(int index) {
                        if (index < 0 || index >= length()) throw new IndexOutOfBoundsException();
                        return pattern.charAt(index % pattern.length());
                    }
                    @Override public CharSequence subSequence(int start, int end) {
                        throw new AssertionError("whole-sequence conversion is not required");
                    }
                    @Override public String toString() {
                        conversions++;
                        StringBuilder value = new StringBuilder(length());
                        for (int index = 0; index < length(); index++) value.append(charAt(index));
                        return value.toString();
                    }
                });
                yield 0;
            }
            case "concurrent", "concurrent_streams" -> {
                out.append(CONCURRENT);
                if (name.equals("concurrent_streams")) err.append("after");
                yield 0;
            }
            default -> 0;
        };
    }
}
