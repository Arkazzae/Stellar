FROM scratch
LABEL org.opencontainers.image.source="https://github.com/Arkazzae/Stellarr" \
      org.opencontainers.image.title="Stellarr browser build inputs"
COPY --link speexdsp-sources.tar.gz /inputs/
COPY --link rnnoise-sources.tar.gz /inputs/
COPY --link rnnoise-data.tar.gz /inputs/
COPY --link gtcrn-sources.tar.gz /inputs/
COPY --link bergamot-sources.tar.gz /inputs/
COPY --link translation-models.tar.gz /inputs/
COPY --link SHA256SUMS NOTICES /inputs/
