ARG BASE_IMAGE=ai-image-studio:local
FROM ${BASE_IMAGE}

USER root
COPY host-updater/tests/fixtures/failure-entrypoint.sh /usr/local/bin/ai-image-studio-drill
RUN chmod 0755 /usr/local/bin/ai-image-studio-drill
USER 10001:10001

ENTRYPOINT ["/usr/local/bin/ai-image-studio-drill"]
CMD ["serve"]
