package io.github.mbackschat.mdrun.sdk;

/** Either permits cooperative execution or supplies a stable nonempty cold-routing reason. */
public sealed interface Classification permits Classification.Eligible, Classification.Refused {
    record Eligible() implements Classification {}

    record Refused(String reason) implements Classification {}
}
